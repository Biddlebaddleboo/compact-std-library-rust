#[test]
fn unsupported_compact_inputs_have_compiler_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
