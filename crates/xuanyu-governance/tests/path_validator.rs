use std::fs;
use std::path::PathBuf;

use xuanyu_governance::SecurePathValidator;

#[test]
fn accepts_paths_under_project_root() {
    let root = std::env::temp_dir().join(format!("xy-path-validator-{}", std::process::id()));
    let _ = fs::create_dir_all(&root);
    let file = root.join("readme.md");
    fs::write(&file, "ok").unwrap();
    let validator = SecurePathValidator::new(&root);
    assert!(validator.validate(&file).valid);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn rejects_paths_outside_project_root() {
    let root = std::env::temp_dir().join(format!("xy-path-validator-root-{}", std::process::id()));
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    // 不能把对照路径放在 tmpdir 下：tmpdir 本身就是允许根。
    let other = home.join(format!("xy-path-validator-other-{}", std::process::id()));
    let _ = fs::create_dir_all(&root);
    let _ = fs::create_dir_all(&other);
    let file = other.join("secret.txt");
    fs::write(&file, "no").unwrap();
    let validator = SecurePathValidator::new(&root);
    assert!(!validator.validate(&file).valid);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&other);
}
