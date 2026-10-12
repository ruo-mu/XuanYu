use xuanyu_governance::{
    classify_git_segment, command_has_destructive_git, GitCommandClass,
};

#[test]
fn force_push_is_destructive() {
    assert_eq!(
        classify_git_segment("git push --force origin main"),
        Some(GitCommandClass::Destructive)
    );
    assert!(command_has_destructive_git("npm test && git push -f"));
}

#[test]
fn hard_reset_is_destructive() {
    assert_eq!(
        classify_git_segment("git reset --hard HEAD~1"),
        Some(GitCommandClass::Destructive)
    );
}

#[test]
fn status_is_read() {
    assert_eq!(
        classify_git_segment("git status"),
        Some(GitCommandClass::Read)
    );
}
