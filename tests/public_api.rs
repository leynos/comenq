//! Compile-time coverage for the public client and daemon configuration APIs.

#[test]
fn changed_public_apis_compile_as_documented() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/pass/*.rs");
    cases.compile_fail("tests/ui/fail/*.rs");
}
