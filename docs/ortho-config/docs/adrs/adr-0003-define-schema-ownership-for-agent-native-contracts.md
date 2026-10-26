# Architectural decision record (ADR) 0003: Define schema ownership for agent-native contracts

Status: Accepted

Date: 2026-05-20

## Context and problem statement

OrthoConfig is adding agent-native command support in phases. The existing
documentation intermediate representation (IR) is localized and oriented
towards human documentation. Future work also needs compact agent invocation
context and machine-readable policy reports from `cargo-orthohelp`.

Those three contracts have different audiences, versioning pressure, and
runtime boundaries. Without an explicit ownership model, localized prose,
agent-facing command facts, and command-line policy diagnostics could drift
into one schema and make every compatibility change more expensive than it
needs to be.

## Decision drivers

- Preserve the current `OrthoConfigDocs` derive contract for human
  documentation.
- Keep machine-readable agent context compact and independently versioned.
- Keep `cargo-orthohelp` command policy reporting close to the tool that emits
  warnings and hard failures.
- Avoid making `ortho_config` depend on Cargo metadata loading, process I/O,
  filesystem writing, or renderer details.
- Let future roadmap items add transforms and command flags without reopening
  the basic ownership boundary.

## Considered options

### Option 1: Put every schema in `OrthoConfigDocs`

This would make one public trait the source for documentation, agent context,
and policy reports.

The option was rejected. It would couple localized prose and renderer-oriented
metadata to compact agent invocation context. It would also make tool-owned
policy diagnostics part of the reusable library contract before there is a
downstream need for that extraction.

### Option 2: Let `cargo-orthohelp` own every schema

This would let the CLI tool control all generated outputs, including the
agent-context contract.

The option was rejected. Downstream applications need a reusable agent-context
schema that does not require depending on `cargo-orthohelp` as a library. The
CLI tool should transform and emit outputs, but it should not own reusable
command-contract types that belong to consumers.

### Option 3: Split ownership by contract audience

The documentation IR remains in `ortho_config::docs`. Agent context is a
sibling reusable contract in `ortho_config::agent_context`. Policy reports are
owned by `cargo-orthohelp::policy` until a later decision extracts a reusable
report model.

This is the accepted option.

## Decision outcome

`ortho_config::docs` continues to own localized documentation IR through
`OrthoConfigDocs`, `DocMetadata`, and `ORTHO_DOCS_IR_VERSION`.

`ortho_config::agent_context` owns compact agent-context schema types and
`ORTHO_AGENT_CONTEXT_SCHEMA_VERSION`. The schema models machine-oriented
command invocation facts and applies explicit defaults for legacy or absent
metadata. It does not contain Fluent identifiers, roff output, PowerShell help
structures, or localized long prose.

`cargo-orthohelp::policy` owns policy-report schema types and
`ORTHO_POLICY_REPORT_SCHEMA_VERSION`. Reports include the emitting tool,
selected mode, stable rule identifiers, machine-readable finding codes,
severities, messages, optional source locations, and severity summaries.

`cargo-orthohelp` remains the reference adapter that builds bridge IR,
localizes human documentation, and later transforms metadata into agent context
or policy reports. The reusable contracts point inward; process execution,
Cargo metadata, filesystem writes, stdout, stderr, and generated artefacts stay
at the adapter boundary.

## Consequences

Documentation IR, agent context, and policy reports can evolve independently. A
human-documentation compatibility change does not force an agent-context
version bump unless the same machine contract changes.

Existing human-documentation outputs remain compatible until a separate
versioned migration is approved. Adding agent-context metadata, JSON result
streams, or policy reports does not change the accepted `cargo-orthohelp` `ir`,
`man`, `ps`, or `all` formats, generated file paths, or process success/failure
contract unless that migration explicitly updates the surface. Roadmap item
6.2.2 is such a migration for `--format all`: it adds `agent-context.json`
beside the existing IR, man-page, and PowerShell artefacts without changing
their paths or meanings. The agent-context versioning and compatibility
mechanism is defined in
[agent-native-cli-design.md](../agent-native-cli-design.md) §8.2.

Issue #329 establishes an additive stdout channel for the two agent-native
contracts: `--format agent-context` and `--format all` emit the generated
AgentContext JSON while retaining `agent-context.json`, and
`--check-agent-native` emits the PolicyReport JSON while retaining
`policy-report.json`. The output adapter owns serialization and stream writes;
the schema owners and wire versions are unchanged. When policy checking and an
explicit agent-context format are combined, the policy report is the only JSON
document written to stdout.

New optional metadata fields require explicit defaults for older derives. Those
defaults are applied by OrthoConfig readers, generators, or transforms; schema
annotations may document defaults but do not populate missing values during
validation.

Future roadmap work that adds `--format agent-context`, `--check-agent-native`,
JSON result streams, or policy evaluation must build on these contracts instead
of scraping rendered help output.

The authoritative consumer dependency tier for Weaver, Netsuke, and other
downstream consumers is recorded in
[agent-native-cli-design.md](../agent-native-cli-design.md) §2.2.

Static Analysis Results Interchange Format (SARIF), JSON Schema, and Model
Context Protocol tool definitions remain useful prior art for field naming and
machine-readable structure. They are not compatibility targets for this
decision.
