use std::path::PathBuf;

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

pub fn examples_root() -> PathBuf {
    workspace_root().join("examples").join("scripts-rhai")
}

pub fn example_dir(name: &str) -> PathBuf {
    examples_root().join(name)
}

pub fn testcase_path(name: &str) -> PathBuf {
    example_dir(name).join("testcase.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_points_to_workspace() {
        assert!(workspace_root().join("Cargo.toml").exists());
    }

    #[test]
    fn examples_root_points_to_examples_directory() {
        assert!(examples_root().is_dir());
    }

    #[test]
    fn example_dir_joins_name() {
        assert!(example_dir("01-text-code").is_dir());
    }

    #[test]
    fn testcase_path_joins_default_filename() {
        let path = testcase_path("01-text-code");
        assert!(path.ends_with("testcase.json"));
    }
}
