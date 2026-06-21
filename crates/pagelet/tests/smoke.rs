use pagelet::{build_info, engine::Engine};

#[test]
fn public_facade_is_constructible() {
    assert_eq!(build_info().crate_name, "pagelet");
    assert_eq!(Engine::new(), Engine);
}
