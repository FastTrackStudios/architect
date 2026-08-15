#[test]
fn cn_macro_is_public() {
    assert_eq!(architect_ui::cn!("px-2 py-1", "py-3"), "px-2 py-3");
}

#[test]
fn cn_macro_accepts_conditionals() {
    let selected = true;
    let disabled = false;

    assert_eq!(
        architect_ui::cn!(
            "inline-flex px-2",
            (selected, "bg-primary"),
            (disabled, "opacity-50"),
            Some("px-4"),
        ),
        "inline-flex bg-primary px-4"
    );
}
