use super::lifecycle::{CompletionKind, PendingBoundary, RuntimeFrame};
use super::*;

impl ScriptLangEngine {
    pub fn choose(&mut self, index: usize) -> Result<(), ScriptLangError> {
        let Some(pending) = self.pending_boundary.take() else {
            return Err(ScriptLangError::new(
                "ENGINE_NO_PENDING_CHOICE",
                "No pending choice is available.",
            ));
        };

        let (frame_id, node_id, options, prompt_text) = match pending {
            PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            } => (frame_id, node_id, options, prompt_text),
            other => {
                self.pending_boundary = Some(other);
                return Err(ScriptLangError::new(
                    "ENGINE_NO_PENDING_CHOICE",
                    "No pending choice is available.",
                ));
            }
        };

        if index >= options.len() {
            self.pending_boundary = Some(PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            });
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_INDEX",
                format!("Choice index \"{}\" is out of range.", index),
            ));
        }

        let Some(frame_index) = self.find_frame_index(frame_id) else {
            self.pending_boundary = Some(PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            });
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_FRAME_MISSING",
                "Pending choice frame is missing.",
            ));
        };

        let node_index = self.frames[frame_index].node_index;
        let group_id = self.frames[frame_index].group_id.clone();
        let (script_name, group) = match self.lookup_group(&group_id) {
            Ok(found) => found,
            Err(error) => {
                self.pending_boundary = Some(PendingBoundary::Choice {
                    frame_id,
                    node_id,
                    options,
                    prompt_text,
                });
                return Err(error);
            }
        };

        let Some(ScriptNode::Choice {
            options: node_options,
            ..
        }) = group.nodes.get(node_index)
        else {
            self.pending_boundary = Some(PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            });
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_NODE_MISSING",
                "Pending choice node is no longer valid.",
            ));
        };

        let item = &options[index];
        let Some(option) = node_options
            .iter()
            .find(|candidate| candidate.id == item.id)
            .cloned()
        else {
            self.pending_boundary = Some(PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            });
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_NOT_FOUND",
                "Choice option no longer exists.",
            ));
        };
        let script_name = script_name.to_string();

        let next_node_index = self.frames[frame_index]
            .node_index
            .checked_add(1)
            .expect("node index should not overflow");
        if let Err(error) =
            self.push_group_frame(&option.group_id, CompletionKind::ResumeAfterChild)
        {
            self.pending_boundary = Some(PendingBoundary::Choice {
                frame_id,
                node_id,
                options,
                prompt_text,
            });
            return Err(error);
        }
        self.frames[frame_index].node_index = next_node_index;
        if option.once {
            self.mark_once_state(&script_name, &format!("option:{}", option.id));
        }
        self.waiting_choice = false;
        Ok(())
    }

    pub fn submit_input(&mut self, text: &str) -> Result<(), ScriptLangError> {
        let Some(pending) = self.pending_boundary.take() else {
            return Err(ScriptLangError::new(
                "ENGINE_NO_PENDING_INPUT",
                "No pending input is available.",
            ));
        };

        let (frame_id, node_id, target_var, prompt_text, default_text) = match pending {
            PendingBoundary::Input {
                frame_id,
                node_id,
                target_var,
                prompt_text,
                default_text,
            } => (frame_id, node_id, target_var, prompt_text, default_text),
            other => {
                self.pending_boundary = Some(other);
                return Err(ScriptLangError::new(
                    "ENGINE_NO_PENDING_INPUT",
                    "No pending input is available.",
                ));
            }
        };

        let Some(frame_index) = self.find_frame_index(frame_id) else {
            self.pending_boundary = Some(PendingBoundary::Input {
                frame_id,
                node_id,
                target_var,
                prompt_text,
                default_text,
            });
            return Err(ScriptLangError::new(
                "ENGINE_INPUT_FRAME_MISSING",
                "Pending input frame is missing.",
            ));
        };

        let normalized = if text.trim().is_empty() {
            default_text.clone()
        } else {
            text.to_string()
        };

        if let Err(error) = self.write_path(&target_var, SlValue::String(normalized)) {
            self.pending_boundary = Some(PendingBoundary::Input {
                frame_id,
                node_id,
                target_var,
                prompt_text,
                default_text,
            });
            return Err(error);
        }

        self.frames[frame_index].node_index += 1;
        self.waiting_choice = false;
        Ok(())
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::runtime_test_support::*;
    use super::*;

    #[test]
    pub(super) fn choose_and_input_validate_pending_boundary_state() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let choose_error = engine.choose(0).expect_err("no pending choice");
        assert_eq!(choose_error.code, "ENGINE_NO_PENDING_CHOICE");
        let input_error = engine.submit_input("x").expect_err("no pending input");
        assert_eq!(input_error.code, "ENGINE_NO_PENDING_INPUT");
    }

    #[test]
    pub(super) fn choose_rejects_out_of_range_index() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let error = engine.choose(9).expect_err("index out of range");
        assert_eq!(error.code, "ENGINE_CHOICE_INDEX");
    }

    #[test]
    pub(super) fn submit_input_uses_default_value_for_blank_input() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("   ").expect("submit input");
        let second = engine.next_output().expect("next");
        assert!(matches!(second, EngineOutput::Text { ref text } if text == "Hello Traveler"));
    }

    #[test]
    pub(super) fn submit_input_uses_provided_non_empty_value() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("Guild").expect("submit input");
        let second = engine.next_output().expect("next");
        assert!(matches!(second, EngineOutput::Text { ref text } if text == "Hello Guild"));
    }

    #[test]
    pub(super) fn choose_restores_pending_boundary_on_internal_failures() {
        let mut wrong_kind = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        wrong_kind.pending_boundary = Some(PendingBoundary::Input {
            frame_id: 1,
            node_id: "i".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        });
        let error = wrong_kind
            .choose(0)
            .expect_err("wrong boundary kind should fail");
        assert_eq!(error.code, "ENGINE_NO_PENDING_CHOICE");
        assert!(matches!(
            wrong_kind.pending_boundary,
            Some(PendingBoundary::Input { .. })
        ));

        let mut lookup_fail = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        lookup_fail.start("main", None).expect("start");
        assert!(matches!(
            lookup_fail.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let frame_index = lookup_fail
            .find_frame_index(lookup_fail.top_frame_id().expect("frame"))
            .expect("frame index");
        lookup_fail.frames[frame_index].group_id = "missing-group".to_string();
        let error = lookup_fail
            .choose(0)
            .expect_err("lookup failure should keep boundary");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");
        assert!(matches!(
            lookup_fail.pending_boundary,
            Some(PendingBoundary::Choice { .. })
        ));

        let mut node_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        node_missing.start("main", None).expect("start");
        assert!(matches!(
            node_missing.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let frame_index = node_missing
            .find_frame_index(node_missing.top_frame_id().expect("frame"))
            .expect("frame index");
        node_missing.frames[frame_index].node_index = 99;
        let error = node_missing
            .choose(0)
            .expect_err("node missing should keep boundary");
        assert_eq!(error.code, "ENGINE_CHOICE_NODE_MISSING");
        assert!(matches!(
            node_missing.pending_boundary,
            Some(PendingBoundary::Choice { .. })
        ));

        let mut option_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        option_missing.start("main", None).expect("start");
        assert!(matches!(
            option_missing.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let pending = option_missing
            .pending_boundary
            .as_mut()
            .expect("pending choice should exist");
        assert!(matches!(pending, PendingBoundary::Choice { .. }));
        if let PendingBoundary::Choice { options, .. } = pending {
            options[0].id = "missing".to_string();
        }
        let error = option_missing
            .choose(0)
            .expect_err("option missing should keep boundary");
        assert_eq!(error.code, "ENGINE_CHOICE_NOT_FOUND");
        assert!(matches!(
            option_missing.pending_boundary,
            Some(PendingBoundary::Choice { .. })
        ));

        let mut push_fail = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" once="true"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        push_fail.start("main", None).expect("start");
        assert!(matches!(
            push_fail.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let frame_id = push_fail.top_frame_id().expect("frame");
        let frame_index = push_fail.find_frame_index(frame_id).expect("frame index");
        let before_node_index = push_fail.frames[frame_index].node_index;
        let script_name = push_fail
            .resolve_current_script_name()
            .expect("script name should resolve");
        let pending = push_fail
            .pending_boundary
            .as_ref()
            .expect("pending choice should exist");
        assert!(matches!(pending, PendingBoundary::Choice { .. }));
        let mut once_key = None;
        if let PendingBoundary::Choice { options, .. } = pending {
            once_key = Some(format!("option:{}", options[0].id));
        }
        let once_key = once_key.expect("choice options should exist");
        assert!(!push_fail.has_once_state(&script_name, &once_key));
        for script in push_fail.scripts.values_mut() {
            for group in script.groups.values_mut() {
                for node in &mut group.nodes {
                    if let ScriptNode::Choice { options, .. } = node {
                        options[0].group_id = "missing-group".to_string();
                    }
                }
            }
        }
        let error = push_fail
            .choose(0)
            .expect_err("push frame failure should keep boundary");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");
        assert_eq!(push_fail.frames[frame_index].node_index, before_node_index);
        assert!(!push_fail.has_once_state(&script_name, &once_key));
        assert!(matches!(
            push_fail.pending_boundary,
            Some(PendingBoundary::Choice { .. })
        ));
    }

    #[test]
    pub(super) fn submit_input_restores_pending_boundary_on_internal_failures() {
        let mut wrong_kind = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        wrong_kind.pending_boundary = Some(PendingBoundary::Choice {
            frame_id: 1,
            node_id: "c".to_string(),
            options: vec![ChoiceItem {
                index: 0,
                id: "opt".to_string(),
                text: "A".to_string(),
            }],
            prompt_text: None,
        });
        let error = wrong_kind
            .submit_input("x")
            .expect_err("wrong boundary kind should fail");
        assert_eq!(error.code, "ENGINE_NO_PENDING_INPUT");
        assert!(matches!(
            wrong_kind.pending_boundary,
            Some(PendingBoundary::Choice { .. })
        ));

        let mut write_fail = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
    </script>
    "#,
        )]));
        write_fail.start("main", None).expect("start");
        assert!(matches!(
            write_fail.next_output().expect("input"),
            EngineOutput::Input { .. }
        ));
        let pending = write_fail
            .pending_boundary
            .as_mut()
            .expect("pending input should exist");
        assert!(matches!(pending, PendingBoundary::Input { .. }));
        if let PendingBoundary::Input { target_var, .. } = pending {
            *target_var = "missingVar".to_string();
        }
        let error = write_fail
            .submit_input("Guild")
            .expect_err("write path should fail");
        assert_eq!(error.code, "ENGINE_VAR_WRITE");
        assert!(matches!(
            write_fail.pending_boundary,
            Some(PendingBoundary::Input { .. })
        ));
    }
}
