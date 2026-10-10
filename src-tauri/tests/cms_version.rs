//! Edge test: the built `cms` binary reports its version without touching
//! any config.

use std::process::Command;

#[test]
fn test_should_print_cms_version_when_run_with_flag() {
    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_cms"))
            .arg(flag)
            // No config must be needed: point HOME somewhere empty.
            .env("HOME", std::env::temp_dir())
            .output()
            .expect("spawn cms");
        assert!(out.status.success(), "{flag} exited with {:?}", out.status);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            format!("cms {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
