# cargo-orthohelp policy and agent-context API guide

This guide shows library consumers how to construct the two agent-native JSON
contracts. The ownership boundary is defined by
[ADR-0003](adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md):
`cargo_orthohelp` owns policy reports, while `ortho_config` owns reusable agent
context.

The rustdoc, the JSON examples in
[agent-native-cli-design.md](agent-native-cli-design.md) §3.2 and §3.3, and the
committed wire snapshots remain the canonical field references. The
compatibility rules are in
[agent-native-cli-design.md](agent-native-cli-design.md) §8.2. This guide
deliberately demonstrates construction and serialization without duplicating
those field references.

The examples assume compatible `cargo-orthohelp`, `ortho_config`, and
`serde_json` dependencies.

## Policy reports

Use `cargo_orthohelp::policy` to describe the outcome of one policy evaluation.
`PolicyReport::empty(mode)` creates a report with the current
`ORTHO_POLICY_REPORT_SCHEMA_VERSION`, `cargo-orthohelp` as its tool, no
results, and a zeroed summary. `PolicyReport::with_results(mode, results)`
creates the same report and derives its summary from `results`.

<!-- tested-example: api-guide-policy-report-constructors -->
```rust
use cargo_orthohelp::policy::{PolicyMode, PolicyReport};

fn main() {
    let report = PolicyReport::empty(PolicyMode::Warn);
    assert!(report.results.is_empty());
    assert_eq!(report.summary.total, 0);
}
```

Keep `results` and `summary` synchronized. `with_results` calls
`PolicySummary::from_results`, but the public fields can drift when callers
mutate them independently. Construct a replacement report with `with_results`
after changing results rather than editing counters by hand.

Build each `PolicyResult` as a struct literal. Set `location` to `None` when no
source is available. For a file without a precise range, use
`Some(SourceLocation { file, range: None })`; otherwise, populate the nested
`SourceLocation`, `SourceRange`, and `SourcePosition` literals. Both `line` and
`column` are one-based.

`PolicySummary::from_results` increments `off`, `warn`, and `deny` for the
matching `PolicySeverity`. Its `total` is always the number of results, not the
sum of only warning and denial findings.

`ORTHO_POLICY_REPORT_SCHEMA_VERSION` serializes as the top-level `version`
field. `cargo_orthohelp::policy::PolicyMode` and
`ortho_config::agent_context::PolicyMode` are distinct Rust enums with the same
wire values. The policy report model remains in `cargo_orthohelp` until a later
ADR extracts it.

### Serialize a report

The following complete example builds a deny-level result with a source range,
derives the summary, and prints formatted JSON.

<!-- tested-example: api-guide-policy-report-json -->
```rust
use cargo_orthohelp::policy::{
    PolicyMode, PolicyReport, PolicyResult, PolicySeverity, SourceLocation, SourcePosition,
    SourceRange,
};

fn main() -> Result<(), serde_json::Error> {
    let results = vec![PolicyResult {
        rule_id: "agent-context-command-summary".to_owned(),
        code: "missing_summary".to_owned(),
        severity: PolicySeverity::Deny,
        message: "Declare a concise command summary.".to_owned(),
        location: Some(SourceLocation {
            file: "src/cli.rs".to_owned(),
            range: Some(SourceRange {
                start: SourcePosition {
                    line: 42,
                    column: 5,
                },
                end: SourcePosition {
                    line: 42,
                    column: 18,
                },
            }),
        }),
    }];
    let report = PolicyReport::with_results(PolicyMode::Deny, results);

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
```

## Agent context

Use `ortho_config::agent_context` to describe an invocable command surface.
`AgentContext::new(package)` accepts the caller-supplied package name and
creates empty `commands` and `skill_manifests`, default `profiles`, `feedback`,
and `policy`, the current `ORTHO_AGENT_CONTEXT_SCHEMA_VERSION`, and a `kind`
from `agent_context_kind`. Do not hand-format the discriminator:
`AGENT_CONTEXT_KIND_SUFFIX` and `agent_context_kind` define it.

<!-- tested-example: api-guide-agent-context-constructor -->
```rust
use ortho_config::AgentContext;

fn main() {
    let context = AgentContext::new("acme");
    assert_eq!(context.package, "acme");
    assert!(context.commands.is_empty());
}
```

Callers supply `package`, the declared commands, and any non-default capability
metadata. `AgentCommand` has no constructor, so build it as a struct literal.
Either assign the complete command set with `context.commands = vec![...]`, or
append an entry with `context.commands.push(...)`. When absent, `summary` is
omitted from JSON; the other optional `AgentCommand` fields serialize as `null`.

`ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` serializes as the top-level
`schema_version` field. This differs from the policy report's top-level
`version` field.

### Serialize a context

This example shows both command-population patterns and prints the complete
context. The first command has a summary, while the appended command shows the
intentional omission of `summary` and `null` optional fields.

<!-- tested-example: api-guide-agent-context-json -->
```rust
use ortho_config::{
    AgentCommand, AgentContext, InteractionMode, MutationEffect,
};

fn main() -> Result<(), serde_json::Error> {
    let mut context = AgentContext::new("acme");
    let describe_command = AgentCommand {
        path: vec!["acme".to_owned(), "context".to_owned()],
        summary: Some("Describe the command surface.".to_owned()),
        canonical_verb: Some("get".to_owned()),
        inputs: Vec::new(),
        output_modes: vec!["json".to_owned()],
        interaction_mode: InteractionMode::NonInteractive,
        mutation_effect: MutationEffect::ReadOnly,
        bypass_flag: None,
        dry_run_flag: None,
        async_submission: None,
        delivery_route: None,
        pagination: None,
        examples: Vec::new(),
    };

    context.commands = vec![describe_command];
    context.commands.push(AgentCommand {
        path: vec!["acme".to_owned(), "status".to_owned()],
        summary: None,
        canonical_verb: None,
        inputs: Vec::new(),
        output_modes: Vec::new(),
        interaction_mode: InteractionMode::NonInteractive,
        mutation_effect: MutationEffect::ReadOnly,
        bypass_flag: None,
        dry_run_flag: None,
        async_submission: None,
        delivery_route: None,
        pagination: None,
        examples: Vec::new(),
    });

    println!("{}", serde_json::to_string_pretty(&context)?);
    Ok(())
}
```
