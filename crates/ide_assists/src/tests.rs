mod generated;

use expect_test::expect;
use hir::Semantics;
use ide_db::{
    base_db::{fixture::WithFixture, FileId, FileRange, SourceDatabaseExt},
    helpers::{
        insert_use::{ImportGranularity, InsertUseConfig},
        SnippetCap,
    },
    source_change::FileSystemEdit,
    RootDatabase,
};
use stdx::{format_to, trim_indent};
use syntax::TextRange;
use test_utils::{assert_eq_text, extract_offset};

use crate::{
    handlers::Handler, Assist, AssistConfig, AssistContext, AssistKind, AssistResolveStrategy,
    Assists, SingleResolve,
};

pub(crate) const TEST_CONFIG: AssistConfig = AssistConfig {
    snippet_cap: SnippetCap::new(true),
    allowed: None,
    insert_use: InsertUseConfig {
        granularity: ImportGranularity::Crate,
        prefix_kind: hir::PrefixKind::Plain,
        enforce_granularity: true,
        group: true,
    },
};

pub(crate) fn with_single_file(text: &str) -> (RootDatabase, FileId) {
    RootDatabase::with_single_file(text)
}

pub(crate) fn check_assist(assist: Handler, ra_fixture_before: &str, ra_fixture_after: &str) {
    let ra_fixture_after = trim_indent(ra_fixture_after);
    check(assist, ra_fixture_before, ExpectedResult::After(&ra_fixture_after), None);
}

// There is no way to choose what assist within a group you want to test against,
// so this is here to allow you choose.
pub(crate) fn check_assist_by_label(
    assist: Handler,
    ra_fixture_before: &str,
    ra_fixture_after: &str,
    label: &str,
) {
    let ra_fixture_after = trim_indent(ra_fixture_after);
    check(assist, ra_fixture_before, ExpectedResult::After(&ra_fixture_after), Some(label));
}

// FIXME: instead of having a separate function here, maybe use
// `extract_ranges` and mark the target as `<target> </target>` in the
// fixture?
#[track_caller]
pub(crate) fn check_assist_target(assist: Handler, ra_fixture: &str, target: &str) {
    check(assist, ra_fixture, ExpectedResult::Target(target), None);
}

#[track_caller]
pub(crate) fn check_assist_not_applicable(assist: Handler, ra_fixture: &str) {
    check(assist, ra_fixture, ExpectedResult::NotApplicable, None);
}

/// Check assist in unresolved state. Useful to check assists for lazy computation.
#[track_caller]
pub(crate) fn check_assist_unresolved(assist: Handler, ra_fixture: &str) {
    check(assist, ra_fixture, ExpectedResult::Unresolved, None);
}

#[track_caller]
fn check_doc_test(assist_id: &str, before: &str, after: &str) {
    let after = trim_indent(after);
    let (db, file_id, selection) = RootDatabase::with_range_or_offset(&before);
    let before = db.file_text(file_id).to_string();
    let frange = FileRange { file_id, range: selection.into() };

    let assist = Assist::get(&db, &TEST_CONFIG, AssistResolveStrategy::All, frange)
        .into_iter()
        .find(|assist| assist.id.0 == assist_id)
        .unwrap_or_else(|| {
            panic!(
                "\n\nAssist is not applicable: {}\nAvailable assists: {}",
                assist_id,
                Assist::get(&db, &TEST_CONFIG, AssistResolveStrategy::None, frange)
                    .into_iter()
                    .map(|assist| assist.id.0)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });

    let actual = {
        let source_change =
            assist.source_change.expect("Assist did not contain any source changes");
        let mut actual = before;
        if let Some(source_file_edit) = source_change.get_source_edit(file_id) {
            source_file_edit.apply(&mut actual);
        }
        actual
    };
    assert_eq_text!(&after, &actual);
}

enum ExpectedResult<'a> {
    NotApplicable,
    Unresolved,
    After(&'a str),
    Target(&'a str),
}

#[track_caller]
fn check(handler: Handler, before: &str, expected: ExpectedResult, assist_label: Option<&str>) {
    let (db, file_with_caret_id, range_or_offset) = RootDatabase::with_range_or_offset(before);
    let text_without_caret = db.file_text(file_with_caret_id).to_string();

    let frange = FileRange { file_id: file_with_caret_id, range: range_or_offset.into() };

    let sema = Semantics::new(&db);
    let config = TEST_CONFIG;
    let ctx = AssistContext::new(sema, &config, frange);
    let resolve = match expected {
        ExpectedResult::Unresolved => AssistResolveStrategy::None,
        _ => AssistResolveStrategy::All,
    };
    let mut acc = Assists::new(&ctx, resolve);
    handler(&mut acc, &ctx);
    let mut res = acc.finish();

    let assist = match assist_label {
        Some(label) => res.into_iter().find(|resolved| resolved.label == label),
        None => res.pop(),
    };

    match (assist, expected) {
        (Some(assist), ExpectedResult::After(after)) => {
            let source_change =
                assist.source_change.expect("Assist did not contain any source changes");
            assert!(!source_change.source_file_edits.is_empty());
            let skip_header = source_change.source_file_edits.len() == 1
                && source_change.file_system_edits.len() == 0;

            let mut buf = String::new();
            for (file_id, edit) in source_change.source_file_edits {
                let mut text = db.file_text(file_id).as_ref().to_owned();
                edit.apply(&mut text);
                if !skip_header {
                    let sr = db.file_source_root(file_id);
                    let sr = db.source_root(sr);
                    let path = sr.path_for_file(&file_id).unwrap();
                    format_to!(buf, "//- {}\n", path)
                }
                buf.push_str(&text);
            }

            for file_system_edit in source_change.file_system_edits {
                if let FileSystemEdit::CreateFile { dst, initial_contents } = file_system_edit {
                    let sr = db.file_source_root(dst.anchor);
                    let sr = db.source_root(sr);
                    let mut base = sr.path_for_file(&dst.anchor).unwrap().clone();
                    base.pop();
                    let created_file_path = format!("{}{}", base.to_string(), &dst.path[1..]);
                    format_to!(buf, "//- {}\n", created_file_path);
                    buf.push_str(&initial_contents);
                }
            }

            assert_eq_text!(after, &buf);
        }
        (Some(assist), ExpectedResult::Target(target)) => {
            let range = assist.target;
            assert_eq_text!(&text_without_caret[range], target);
        }
        (Some(assist), ExpectedResult::Unresolved) => assert!(
            assist.source_change.is_none(),
            "unresolved assist should not contain source changes"
        ),
        (Some(_), ExpectedResult::NotApplicable) => panic!("assist should not be applicable!"),
        (None, ExpectedResult::After(_))
        | (None, ExpectedResult::Target(_))
        | (None, ExpectedResult::Unresolved) => {
            panic!("code action is not applicable")
        }
        (None, ExpectedResult::NotApplicable) => (),
    };
}

fn labels(assists: &[Assist]) -> String {
    let mut labels = assists
        .iter()
        .map(|assist| {
            let mut label = match &assist.group {
                Some(g) => g.0.clone(),
                None => assist.label.to_string(),
            };
            label.push('\n');
            label
        })
        .collect::<Vec<_>>();
    labels.dedup();
    labels.into_iter().collect::<String>()
}

#[test]
fn assist_order_field_struct() {
    let before = "struct Foo { $0bar: u32 }";
    let (before_cursor_pos, before) = extract_offset(before);
    let (db, file_id) = with_single_file(&before);
    let frange = FileRange { file_id, range: TextRange::empty(before_cursor_pos) };
    let assists = Assist::get(&db, &TEST_CONFIG, AssistResolveStrategy::None, frange);
    let mut assists = assists.iter();

    assert_eq!(assists.next().expect("expected assist").label, "Change visibility to pub(crate)");
    assert_eq!(assists.next().expect("expected assist").label, "Generate `Deref` impl using `bar`");
    assert_eq!(assists.next().expect("expected assist").label, "Generate a mut getter method");
    assert_eq!(assists.next().expect("expected assist").label, "Generate a getter method");
    assert_eq!(assists.next().expect("expected assist").label, "Generate a setter method");
    assert_eq!(assists.next().expect("expected assist").label, "Add `#[derive]`");
}

#[test]
fn assist_order_if_expr() {
    let (db, frange) = RootDatabase::with_range(
        r#"
pub fn test_some_range(a: int) -> bool {
    if let 2..6 = $05$0 {
        true
    } else {
        false
    }
}
"#,
    );

    let assists = Assist::get(&db, &TEST_CONFIG, AssistResolveStrategy::None, frange);
    let expected = labels(&assists);

    expect![[r#"
        Convert integer base
        Extract into variable
        Extract into function
        Replace with match
    "#]]
    .assert_eq(&expected);
}

#[test]
fn assist_filter_works() {
    let (db, frange) = RootDatabase::with_range(
        r#"
pub fn test_some_range(a: int) -> bool {
    if let 2..6 = $05$0 {
        true
    } else {
        false
    }
}
"#,
    );
    {
        let mut cfg = TEST_CONFIG;
        cfg.allowed = Some(vec![AssistKind::Refactor]);

        let assists = Assist::get(&db, &cfg, AssistResolveStrategy::None, frange);
        let expected = labels(&assists);

        expect![[r#"
            Convert integer base
            Extract into variable
            Extract into function
            Replace with match
        "#]]
        .assert_eq(&expected);
    }

    {
        let mut cfg = TEST_CONFIG;
        cfg.allowed = Some(vec![AssistKind::RefactorExtract]);
        let assists = Assist::get(&db, &cfg, AssistResolveStrategy::None, frange);
        let expected = labels(&assists);

        expect![[r#"
            Extract into variable
            Extract into function
        "#]]
        .assert_eq(&expected);
    }

    {
        let mut cfg = TEST_CONFIG;
        cfg.allowed = Some(vec![AssistKind::QuickFix]);
        let assists = Assist::get(&db, &cfg, AssistResolveStrategy::None, frange);
        let expected = labels(&assists);

        expect![[r#""#]].assert_eq(&expected);
    }
}

#[test]
fn various_resolve_strategies() {
    let (db, frange) = RootDatabase::with_range(
        r#"
pub fn test_some_range(a: int) -> bool {
    if let 2..6 = $05$0 {
        true
    } else {
        false
    }
}
"#,
    );

    let mut cfg = TEST_CONFIG;
    cfg.allowed = Some(vec![AssistKind::RefactorExtract]);

    {
        let assists = Assist::get(&db, &cfg, AssistResolveStrategy::None, frange);
        assert_eq!(2, assists.len());
        let mut assists = assists.into_iter();

        let extract_into_variable_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_variable",
                    RefactorExtract,
                ),
                label: "Extract into variable",
                group: None,
                target: 59..60,
                source_change: None,
            }
        "#]]
        .assert_debug_eq(&extract_into_variable_assist);

        let extract_into_function_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_function",
                    RefactorExtract,
                ),
                label: "Extract into function",
                group: None,
                target: 59..60,
                source_change: None,
            }
        "#]]
        .assert_debug_eq(&extract_into_function_assist);
    }

    {
        let assists = Assist::get(
            &db,
            &cfg,
            AssistResolveStrategy::Single(SingleResolve {
                assist_id: "SOMETHING_MISMATCHING".to_string(),
                assist_kind: AssistKind::RefactorExtract,
            }),
            frange,
        );
        assert_eq!(2, assists.len());
        let mut assists = assists.into_iter();

        let extract_into_variable_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_variable",
                    RefactorExtract,
                ),
                label: "Extract into variable",
                group: None,
                target: 59..60,
                source_change: None,
            }
        "#]]
        .assert_debug_eq(&extract_into_variable_assist);

        let extract_into_function_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_function",
                    RefactorExtract,
                ),
                label: "Extract into function",
                group: None,
                target: 59..60,
                source_change: None,
            }
        "#]]
        .assert_debug_eq(&extract_into_function_assist);
    }

    {
        let assists = Assist::get(
            &db,
            &cfg,
            AssistResolveStrategy::Single(SingleResolve {
                assist_id: "extract_variable".to_string(),
                assist_kind: AssistKind::RefactorExtract,
            }),
            frange,
        );
        assert_eq!(2, assists.len());
        let mut assists = assists.into_iter();

        let extract_into_variable_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_variable",
                    RefactorExtract,
                ),
                label: "Extract into variable",
                group: None,
                target: 59..60,
                source_change: Some(
                    SourceChange {
                        source_file_edits: {
                            FileId(
                                0,
                            ): TextEdit {
                                indels: [
                                    Indel {
                                        insert: "let $0var_name = 5;\n    ",
                                        delete: 45..45,
                                    },
                                    Indel {
                                        insert: "var_name",
                                        delete: 59..60,
                                    },
                                ],
                            },
                        },
                        file_system_edits: [],
                        is_snippet: true,
                    },
                ),
            }
        "#]]
        .assert_debug_eq(&extract_into_variable_assist);

        let extract_into_function_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_function",
                    RefactorExtract,
                ),
                label: "Extract into function",
                group: None,
                target: 59..60,
                source_change: None,
            }
        "#]]
        .assert_debug_eq(&extract_into_function_assist);
    }

    {
        let assists = Assist::get(&db, &cfg, AssistResolveStrategy::All, frange);
        assert_eq!(2, assists.len());
        let mut assists = assists.into_iter();

        let extract_into_variable_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_variable",
                    RefactorExtract,
                ),
                label: "Extract into variable",
                group: None,
                target: 59..60,
                source_change: Some(
                    SourceChange {
                        source_file_edits: {
                            FileId(
                                0,
                            ): TextEdit {
                                indels: [
                                    Indel {
                                        insert: "let $0var_name = 5;\n    ",
                                        delete: 45..45,
                                    },
                                    Indel {
                                        insert: "var_name",
                                        delete: 59..60,
                                    },
                                ],
                            },
                        },
                        file_system_edits: [],
                        is_snippet: true,
                    },
                ),
            }
        "#]]
        .assert_debug_eq(&extract_into_variable_assist);

        let extract_into_function_assist = assists.next().unwrap();
        expect![[r#"
            Assist {
                id: AssistId(
                    "extract_function",
                    RefactorExtract,
                ),
                label: "Extract into function",
                group: None,
                target: 59..60,
                source_change: Some(
                    SourceChange {
                        source_file_edits: {
                            FileId(
                                0,
                            ): TextEdit {
                                indels: [
                                    Indel {
                                        insert: "fun_name()",
                                        delete: 59..60,
                                    },
                                    Indel {
                                        insert: "\n\nfn $0fun_name() -> i32 {\n    5\n}",
                                        delete: 110..110,
                                    },
                                ],
                            },
                        },
                        file_system_edits: [],
                        is_snippet: true,
                    },
                ),
            }
        "#]]
        .assert_debug_eq(&extract_into_function_assist);
    }
}
