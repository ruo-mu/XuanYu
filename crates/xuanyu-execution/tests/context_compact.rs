use xuanyu_execution::{select_compact_keep_index_by_roles, CompactStrategyKind};

#[test]
fn keep_index_preserves_user_rounds() {
    let roles = [
        "user", "assistant", "user", "assistant", "user", "assistant", "user", "assistant",
    ];
    // 4 user rounds; preserve 6 → keep all
    assert_eq!(
        select_compact_keep_index_by_roles(&roles, CompactStrategyKind::Microcompact),
        0
    );
    let many: Vec<&str> = (0..20)
        .flat_map(|i| if i % 2 == 0 { ["user"] } else { ["assistant"] })
        .collect();
    // 10 user rounds; preserve 6 → cut at the 5th user (index of user #5 = round 10-6=4 → index 8)
    let keep = select_compact_keep_index_by_roles(&many, CompactStrategyKind::Microcompact);
    assert_eq!(keep, 8);
    assert_eq!(many[keep], "user");
}
