impl ScriptLangEngine {
    pub fn snapshot(&self) -> Result<SnapshotV3, ScriptLangError> {
        let Some(boundary) = &self.pending_boundary else {
            return Err(ScriptLangError::new(
                "SNAPSHOT_NOT_ALLOWED",
                "snapshot() is only allowed while waiting for a choice or input.",
            ));
        };

        let runtime_frames = self
            .frames
            .iter()
            .map(|frame| SnapshotFrameV3 {
                frame_id: frame.frame_id,
                group_id: frame.group_id.clone(),
                node_index: frame.node_index,
                scope: frame.scope.clone(),
                var_types: frame.var_types.clone(),
                completion: match frame.completion {
                    CompletionKind::None => SnapshotCompletion::None,
                    CompletionKind::WhileBody => SnapshotCompletion::WhileBody,
                    CompletionKind::ResumeAfterChild => SnapshotCompletion::ResumeAfterChild,
                },
                script_root: frame.script_root,
                return_continuation: frame.return_continuation.clone(),
            })
            .collect::<Vec<_>>();

        let pending_boundary = match boundary {
            PendingBoundary::Choice {
                node_id,
                options,
                prompt_text,
                ..
            } => PendingBoundaryV3::Choice {
                node_id: node_id.clone(),
                items: options.clone(),
                prompt_text: prompt_text.clone(),
            },
            PendingBoundary::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
                ..
            } => PendingBoundaryV3::Input {
                node_id: node_id.clone(),
                target_var: target_var.clone(),
                prompt_text: prompt_text.clone(),
                default_text: default_text.clone(),
            },
        };

        let once_state_by_script = self
            .once_state_by_script
            .iter()
            .map(|(script_name, set)| {
                let mut values = set.iter().cloned().collect::<Vec<_>>();
                values.sort();
                (script_name.clone(), values)
            })
            .collect();

        Ok(SnapshotV3 {
            schema_version: SNAPSHOT_SCHEMA_V3.to_string(),
            compiler_version: self.compiler_version.clone(),
            runtime_frames,
            rng_state: self.rng_state,
            pending_boundary,
            once_state_by_script,
        })
    }

    pub fn resume(&mut self, snapshot: SnapshotV3) -> Result<(), ScriptLangError> {
        if snapshot.schema_version != SNAPSHOT_SCHEMA_V3 {
            return Err(ScriptLangError::new(
                "SNAPSHOT_SCHEMA",
                format!(
                    "Unsupported snapshot schema \"{}\".",
                    snapshot.schema_version
                ),
            ));
        }

        if snapshot.compiler_version != self.compiler_version {
            return Err(ScriptLangError::new(
                "SNAPSHOT_COMPILER_VERSION",
                format!(
                    "Snapshot compiler version \"{}\" does not match engine \"{}\".",
                    snapshot.compiler_version, self.compiler_version
                ),
            ));
        }

        self.reset();
        self.rng_state = snapshot.rng_state;

        self.once_state_by_script = snapshot
            .once_state_by_script
            .into_iter()
            .map(|(script, entries)| (script, entries.into_iter().collect()))
            .collect();

        self.frames = snapshot
            .runtime_frames
            .into_iter()
            .map(|frame| RuntimeFrame {
                frame_id: frame.frame_id,
                group_id: frame.group_id,
                node_index: frame.node_index,
                scope: frame.scope,
                completion: match frame.completion {
                    SnapshotCompletion::None => CompletionKind::None,
                    SnapshotCompletion::WhileBody => CompletionKind::WhileBody,
                    SnapshotCompletion::ResumeAfterChild => CompletionKind::ResumeAfterChild,
                },
                script_root: frame.script_root,
                return_continuation: frame.return_continuation,
                var_types: frame.var_types,
            })
            .collect();

        self.frame_counter = self
            .frames
            .iter()
            .map(|frame| frame.frame_id)
            .max()
            .unwrap_or(0)
            + 1;

        let top = self
            .frames
            .last()
            .ok_or_else(|| {
                ScriptLangError::new("SNAPSHOT_EMPTY", "Snapshot contains no runtime frames.")
            })?
            .clone();

        let (script_name, group) = self.lookup_group(&top.group_id)?;
        let node = group
            .nodes
            .get(top.node_index)
            .ok_or_else(|| {
                ScriptLangError::new("SNAPSHOT_PENDING_BOUNDARY", "Pending node index invalid.")
            })?
            .clone();

        self.pending_boundary = Some(match snapshot.pending_boundary {
            PendingBoundaryV3::Choice {
                node_id,
                items,
                prompt_text,
            } => {
                let ScriptNode::Choice { id, .. } = node else {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending boundary expects choice node.",
                    ));
                };
                if id != node_id {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending choice node mismatch.",
                    ));
                }
                self.waiting_choice = true;
                PendingBoundary::Choice {
                    frame_id: top.frame_id,
                    node_id,
                    options: items,
                    prompt_text,
                }
            }
            PendingBoundaryV3::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
            } => {
                let ScriptNode::Input { id, .. } = node else {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending boundary expects input node.",
                    ));
                };
                if id != node_id {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending input node mismatch.",
                    ));
                }
                self.waiting_choice = false;
                PendingBoundary::Input {
                    frame_id: top.frame_id,
                    node_id,
                    target_var,
                    prompt_text,
                    default_text,
                }
            }
        });

        // Force visibility map access path to validate script existence.
        let _ = self.visible_json_by_script.get(&script_name);

        Ok(())
    }

}
