# OrthoConfig IR documentation design for cargo-orthohelp (v4)

This document defines the intermediate representation (IR) emitted by the
`OrthoConfig` derive macro and consumed by `cargo-orthohelp` to generate
localized UNIX man pages and PowerShell external help (Microsoft Assistance
Markup Language (MAML)) plus a wrapper module. It focuses on a command-line
interface (CLI) documentation pipeline that remains `clap` agnostic and keeps
IR-driven documentation code out of application binaries. The related
agent-facing invocation contract is defined in
[agent-native-cli-design.md](agent-native-cli-design.md), the canonical
agent-native command-contract and boundary document. It should be emitted as a
sibling output, not as localized documentation prose.

- Status: Revision 4 (clap coverage and contract-evolution amendments
  integrated).
- Audience: OrthoConfig maintainers and consumers.
- Goal: Generate fully localized UNIX man pages and PowerShell external help
  (MAML and wrapper module) from a robust, `clap`-agnostic IR.
- Non-goals:
  - Shipping documentation code in application binaries.
  - Hidden or dummy `clap` arguments.
  - Scraping `--help` output.
  - Depending on `clap_mangen`.

Note: document revisions track narrative changes, while compatibility is
governed by the IR schema version (`ir_version`). Generators must use the IR
schema version to determine compatibility, regardless of document revision.

## 0. Changelog

### 0.1 Revision 4

Revision 4 closes the gap between the IR contract and the ordinary `clap`
vocabulary, and states the evolution rules the published types must obey. None
of these amendments change what an existing consumer already reads; every one
of them widens what the pipeline can describe.

1. The documentation derive must cover the whole ordinary `clap` variant
   vocabulary. Unit variants (`enum Cmd { Start, Stop, Status }`) and
   named-field variants (`Cmd::Remote { #[command(subcommand)] action: … }`)
   are in scope, not deferred. See §3.1.
2. Positional arguments are modelled as first-class CLI surface. `CliMetadata`
   grows optional positional metadata and the derive stops asserting that every
   documented field is a long flag. See §2.2.
3. `OrthoConfigDocs` becomes independently derivable. Documenting a type must
   not require making it loadable from configuration layers. See §3.1.
4. Public IR and agent-context types become `#[non_exhaustive]` and reachable
   through constructors that stamp the schema version, in one coordinated
   breaking release; every subsequent optional metadata addition is then a
   minor release, and no consumer hand-writes `ir_version`. See §3.6 and §12.
5. Enums that carry an `Unknown` variant for forward compatibility wire that
   variant up as the unknown-value fallback, so a reader tolerates the payloads
   the compatibility policy already promises it will tolerate. See §12.
6. The library ships a localizable default heading catalogue rather than
   relying on hardcoded English constants inside the generator. See §4.2.

### 0.2 Revision 3

Revision 3 keeps the human documentation IR intact and adds the agent-native
roadmap boundary:

1. Agent context is a compact sibling output generated from the same metadata
   spine, not a replacement for localized documentation IR.
2. Whole-CLI subcommand metadata is now an explicit dependency for future
   agent-context output. The schema already models recursive subcommands, but
   implementation must populate them before the context is complete.
3. `cargo-orthohelp` becomes the reference CLI for table-stakes agent-native
   behaviour: `--json`, stdout/stderr separation, enumerating errors, stable
   result summaries, and atomic artefact writes.
4. Agent-native linting is now a `cargo-orthohelp` responsibility: the
   `--check-agent-native` flag (with the `--policy-mode <off|warn|deny>`
   override) evaluates the package's policy configuration, writes
   `policy-report.json` atomically to the output directory, and writes a
   one-line summary to stderr. Strict policy, the bridge-driven behaviour
   rules, and future extensions are defined in
   [agent-native-cli-design.md](agent-native-cli-design.md).
5. Consumer applications such as Weaver and Netsuke depend on the same generic
   metadata for renderer policy, JSON mode contracts, exit-code classes, skill
   manifests, context naming, capability provenance, profile redaction,
   delivery and feedback parsers, and configurable execution ledgers.
   [agent-native-cli-design.md](agent-native-cli-design.md) §2.2 is the
   authoritative source for the hard and soft ship-time dependency tier of
   those reusable capabilities.

The existing `ir`, `man`, `ps`, and `all` format compatibility surfaces remain
unchanged by the consumer dependency tier.

### 0.3 Revision 2

PowerShell and Windows amendments (no change to the IR-first philosophy):

1. Wrapper module is mandatory on Windows. `Get-Help` indexes PowerShell
   artefacts only. The generator must emit a module that exports a function
   forwarding to the native executable, and the MAML `<command:name>` must
   match the exported function exactly.
2. Dual module roots: install the same module into both
   `%ProgramFiles%\WindowsPowerShell\Modules\<ModuleName>` (PowerShell 5.1) and
   `%ProgramFiles%\PowerShell\Modules\<ModuleName>` (PowerShell 7+).
3. Completions: register against the wrapper function. At import, detect
   `Register-ArgumentCompleter -Native` (PowerShell 7+) and fall back to the
   non-`-Native` form on PowerShell 5.1.
4. Locale fallback: always generate `en-US/<ModuleName>-help.xml`. If a
   target locale exists but `en-US` does not, copy the target into `en-US` so
   `Get-Help` never returns empty help.
5. CommonParameters: wrappers use
   `[CmdletBinding(PositionalBinding = $false)]` so `Get-Help -Full` lists
   common parameters. The MAML writer includes `<CommonParameters/>` unless
   explicitly disabled.
6. About topic: generate `about_<ModuleName>.help.txt` from the IR conceptual
   material (overview, discovery, precedence) per locale.
7. HelpInfoUri: optional. Only set when Update-Help payloads are published;
   otherwise omit it to avoid broken Update-Help user experience (UX).
8. Microsoft Installer (MSI) layout guidance: place the executable under
   `…\\Program Files\\<Vendor>\\<Product>\\bin\\` and add it to the machine
   PATH, drop the module into both module roots, and recommend code signing for
   the executable and MSI.
9. Wrapper robustness: resolve the executable path relative to
   `$PSScriptRoot`, forward `@Args`, and propagate `$LASTEXITCODE`.
10. IR additions (Windows-only, optional): `windows.module_name` and
    `windows.wrapper` knobs (aliases, common parameters, split subcommands)
    for the generator to consume. These have no runtime impact.

The IR schema bumps to `1.1` to include optional Windows metadata.

## 1. Architecture overview

```plaintext
User crate (uses OrthoConfig)
┌───────────────────────────────────┐
│ #[derive(OrthoConfig)]            │
│ struct AppConfig { … }          │
│                                   │
│ OrthoConfigDocs::get_doc_metadata │
│   -> DocMetadata (IR)             │
└───────────────────────────────────┘

OrthoConfig workspace
┌───────────────────────────────────┐
│ ortho-config (runtime + macros)   │
│ - Localizer + Fluent impl         │
│ - Derive: runtime loaders         │
│ - Derive: OrthoConfigDocs (IR)    │
└───────────────────────────────────┘

cargo-orthohelp (CLI tool)
┌───────────────────────────────────┐
│ 1) Builds ephemeral bridge        │
│ 2) Calls get_doc_metadata()       │
│ 3) Resolves Fluent IDs per locale │
│ 4) Emits roff and MAML + module   │
└───────────────────────────────────┘
```

Key choices:

- IR over `clap` for complete coverage across CLI, environment variables, and
  files without dummy arguments.
- Localization at generation time: the IR stores message identifiers (IDs),
  and generators resolve per locale.
- Out-of-band tooling: `cargo orthohelp` compiles and runs a tiny ephemeral
  bridge to fetch the IR, keeping application binaries clean.
- Sibling agent outputs: compact agent context and policy reports are generated
  from the same metadata spine but versioned independently from localized
  documentation IR. `DocMetadata.ir_version` governs compatibility for human
  documentation IR, while the future agent-context schema version governs
  compact agent-facing output.
- Ownership boundary:
  [ADR-0003](adrs/adr-0003-define-schema-ownership-for-agent-native-contracts.md)
  keeps documentation IR in `ortho_config::docs`, reusable agent context in
  `ortho_config::agent_context`, and policy reports in
  `cargo_orthohelp::policy` until a later extraction is approved.

## 2. Documentation IR (schema v2.0)

### 2.1 Top-level metadata

```rust
#[derive(Debug, Serialize)]
pub struct DocMetadata {
    pub ir_version: String,            // e.g., "2.0"
    pub app_name: String,              // binary or display name
    pub bin_name: Option<String>,      // override for man page or wrapper name
    pub about_id: String,              // Fluent ID for app description
    pub synopsis_id: Option<String>,   // Fluent ID for synopsis summary
    pub sections: SectionsMetadata,    // headings, discovery, precedence, etc.
    pub fields: Vec<FieldMetadata>,    // flattened fields for this command
    pub subcommands: Vec<DocMetadata>, // recursively the same schema
    pub windows: Option<WindowsMetadata>, // Windows-only generator hints
    pub behaviour: Option<BehaviourMetadata>, // declared execution behaviour
}
```

`subcommands` is populated by `OrthoConfigSubcommandDocs` when the root config
contains a clap subcommand selector. Configs without subcommands still emit an
empty vector, preserving the schema shape and version.

```rust
#[derive(Debug, Serialize)]
pub struct SectionsMetadata {
    pub headings_ids: HeadingIds,        // Fluent IDs for standard headings
    pub discovery: Option<ConfigDiscoveryMeta>,
    pub precedence: Option<PrecedenceMeta>,
    pub examples: Vec<Example>,          // app-level examples
    pub links: Vec<Link>,                // app-level related links
    pub notes: Vec<Note>,                // app-level notes or disclaimers
}
```

```rust
#[derive(Debug, Serialize)]
pub struct HeadingIds {
    pub name: String,
    pub synopsis: String,
    pub description: String,
    pub options: String,
    pub environment: String,
    pub files: String,
    pub precedence: String,
    pub exit_status: String,
    pub examples: String,
    pub see_also: String,
    pub commands: Option<String>, // inline subcommand listing
}
```

Every identifier in `HeadingIds` resolves against the library heading catalogue
described in §4.2. A consumer that ships no Fluent catalogue of its own still
renders complete headings; a consumer that ships one overrides only the
headings it cares about.

### 2.2 Field-level metadata

```rust
#[derive(Debug, Serialize)]
pub struct FieldMetadata {
    pub name: String,                  // Rust field name
    pub help_id: String,               // Fluent ID for field help
    pub long_help_id: Option<String>,  // optional long help ID
    pub value: Option<ValueType>,      // semantic value type for rendering
    pub default: Option<DefaultValue>,
    pub required: bool,
    pub deprecated: Option<Deprecation>,
    pub cli: Option<CliMetadata>,      // if exposed via CLI
    pub env: Option<EnvMetadata>,      // if exposed via environment variable
    pub file: Option<FileMetadata>,    // if exposed via files
    pub examples: Vec<Example>,        // field-level examples
    pub links: Vec<Link>,
    pub notes: Vec<Note>,
}
```

```rust
#[derive(Debug, Serialize)]
pub struct CliMetadata {
    pub long: Option<String>,          // "port"; None for positionals
    pub short: Option<char>,           // 'p'
    pub value_name: Option<String>,    // e.g., "NUM"
    pub multiple: bool,                // repeats allowed
    pub takes_value: bool,             // false for switches
    pub possible_values: Vec<String>,  // for enums
    pub hide_in_help: bool,            // excluded from OPTIONS section
    pub positional: Option<PositionalMetadata>, // set for positional arguments
}

#[derive(Debug, Serialize)]
pub struct PositionalMetadata {
    pub index: u16,        // 1-based clap argument index
    pub variadic: bool,    // accepts more than one value
    pub var_arg: bool,     // clap `Arg::trailing_var_arg`: consumes the remaining arguments,
                           // including hyphenated ones, without requiring `--`
    pub last: bool,        // clap `Arg::last`: reachable only through `--`
}

#[derive(Debug, Serialize)]
pub struct EnvMetadata {
    pub var_name: String,
}

#[derive(Debug, Serialize)]
pub struct FileMetadata {
    pub key_path: String, // e.g., "database.host"
}
```

`var_arg` and `last` model orthogonal clap settings and must not be conflated:
a field may set either, both, or neither, and the generated syntax must
represent whichever combination the field actually declares.

A field is positional when `positional` is `Some`. Positional arguments are
ordinary CLI surface — `git clone <url>` and `cp <src> <dst>` are the common
shape — and they must be representable without special-casing. Three rules
follow:

- `long` and `short` are both `None` for a positional argument. The derive must
  not emit a synthesized long flag to keep the field addressable.
- Generators order positionals by `index` in SYNOPSIS output, and by
  `index - 1` in the PowerShell `position` attribute, because clap `index` is
  1-based while PowerShell `Position` is 0-based; they render `value_name`
  rather than a flag spelling.
- The agent-context contract emits a positional input only when `positional`
  is `Some`, ordering emitted positional inputs by the positional metadata's
  `index`. A field with CLI metadata, no flag spelling, and no `positional`
  entry is configuration surface exposed through environment variables or files
  rather than invocation surface, and agent-facing output omits it.

Adding `positional` is an additive optional field: it bumps the IR minor
version and leaves existing `ir`, `man`, and `ps` output unchanged for command
surfaces that use only flags.

### 2.3 Value typing and defaults

```rust
#[derive(Debug, Serialize)]
pub enum ValueType {
    String,
    Integer { bits: u8, signed: bool },
    Float { bits: u8 },
    Bool,
    Duration,
    Path,
    IpAddr,
    Hostname,
    Url,
    Enum { variants: Vec<String> },
    List { of: Box<ValueType> },
    Map { of: Box<ValueType> },
    Custom { name: String },
}

#[derive(Debug, Serialize)]
pub struct DefaultValue {
    pub display: String,
}

#[derive(Debug, Serialize)]
pub struct Deprecation {
    pub note_id: String,
}
```

### 2.4 Config discovery and precedence

```rust
#[derive(Debug, Serialize)]
pub struct ConfigDiscoveryMeta {
    pub formats: Vec<ConfigFormat>,         // e.g., [Toml, Json, Yaml]
    pub search_paths: Vec<PathPattern>,     // ordered (lowest -> highest)
    pub override_flag_long: Option<String>, // e.g., "config"
    pub override_env: Option<String>,       // e.g., "MY_APP_CONFIG"
    pub xdg_compliant: bool,
}

#[derive(Debug, Serialize)]
pub enum ConfigFormat {
    Toml,
    Yaml,
    Json,
}

#[derive(Debug, Serialize)]
pub struct PathPattern {
    pub pattern: String,
    pub note_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PrecedenceMeta {
    pub order: Vec<SourceKind>,             // e.g., [File, Env, Cli]
    pub rationale_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub enum SourceKind {
    Defaults,
    File,
    Env,
    Cli,
}
```

XDG refers to the XDG Base Directory specification used for configuration
discovery and the `xdg_compliant` flag.

### 2.5 Windows metadata (optional)

```rust
#[derive(Debug, Serialize)]
pub struct WindowsMetadata {
    pub module_name: Option<String>,
    pub export_aliases: Vec<String>,
    pub include_common_parameters: bool,
    pub split_subcommands_into_functions: bool,
    pub help_info_uri: Option<String>,
}
```

Resolution order for Windows generator settings: CLI flags override
`Cargo.toml` metadata, which overrides `windows` values from the IR. Defaults
apply when no source provides a value. When multiple sources provide the same
setting, the highest-precedence source wins.

### 2.6 Extras (examples, links, and notes)

```rust
#[derive(Debug, Serialize)]
pub struct Example {
    pub title_id: Option<String>,
    pub code: String,
    pub body_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Link {
    pub text_id: Option<String>,
    pub uri: String,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub text_id: String,
}
```

Flattening rule: config file keys must be emitted as dotted `key_path` (e.g.,
`network.proxy.url`) regardless of internal nesting. Environment variable names
must be unique per field (see section 5.3). The only nested structure is
`subcommands`.

## 3. Derive macro integration

### 3.1 Trait

```rust
pub trait OrthoConfigDocs {
    /// Returns the complete documentation metadata for this config.
    fn get_doc_metadata() -> DocMetadata;
}
```

The `#[derive(OrthoConfig)]` macro emits this implementation alongside runtime
loaders, filling all IR fields from the same parsed metadata.

`OrthoConfigDocs` is also derivable on its own through
`#[derive(OrthoConfigDocs)]`. Documentation and configuration loading are
separate concerns: most argument structs in a subcommand tree are `clap`-only
and are never merged from configuration layers. Requiring `OrthoConfig` in
order to document a type would force those structs to satisfy
`DeserializeOwned` and to carry a `Default` implementation that lies about
required fields, purely to obtain documentation.

The standalone derive therefore:

- shares the `#[derive(OrthoConfig)]` field-metadata pipeline, but drives it
  through a docs-only mode that reads clap's actual argument attributes —
  `long`, `short`, `index`, `value_name`, and the positional settings — instead
  of synthesizing a flag spelling from the field identifier, never invents a
  short flag the type does not declare, and emits environment and file metadata
  only where explicitly declared;
- emits no runtime loaders, no `Deserialize` bound assertion, and no merge
  machinery;
- applies to any `#[derive(clap::Args)]` or `#[derive(clap::Parser)]` struct,
  including one with required fields and no `Default`;
- detects `#[command(subcommand)]` fields, excludes the selector from ordinary
  field metadata, and delegates it to `OrthoConfigSubcommandDocs`, preserving
  the returned nodes recursively in `DocMetadata.subcommands`;
- is mutually exclusive with `#[derive(OrthoConfig)]` on the same type. Both on
  one type is a macro-time error naming the duplicate rather than a confusing
  conflicting-implementation error from the compiler.

Subcommand enums use a companion trait:

```rust
pub trait OrthoConfigSubcommandDocs {
    /// Returns one documentation metadata node per subcommand variant.
    fn get_subcommand_doc_metadata() -> Vec<DocMetadata>;
}
```

The `OrthoConfigSubcommandDocs` derive is applied to `clap::Subcommand` enums
and preserves enum declaration order. It covers the whole ordinary `clap`
variant vocabulary:

- **Single-field tuple variants** (`Run(RunArgs)`) delegate to the inner
  argument struct's `OrthoConfigDocs` implementation, override `app_name` with
  the clap command label, and regenerate `about_id`.
- **Unit variants** (`Start`, `Stop`, `Status`) emit a minimal `DocMetadata`
  node: command label, generated `about_id`, default heading identifiers, and
  empty `fields` and `subcommands`. A service or daemon CLI is built from
  these, so rejecting them excludes an entire ordinary class of command surface.
- **Named-field variants** (`Remote { #[command(subcommand)] action: … }`)
  build their node from the variant's own fields. A `#[command(subcommand)]`
  field recurses through the nested selector enum to produce grandchild nodes;
  remaining fields run through the same field-metadata pipeline as struct
  fields. This is the canonical clap idiom for three-level command trees, and a
  feature whose purpose is recursive subcommand metadata has to describe it.

Variants that mix a nested `#[command(subcommand)]` selector with ordinary
argument fields are supported: the selector populates `subcommands` and the
remaining fields populate `fields`. More than one `#[command(subcommand)]`
field in a single variant is a macro-time error, matching clap's own constraint.

### 3.2 Attributes (doc-related)

Namespace: `#[ortho_config(…)]`. Selected keys:

- IDs and text: `help_id`, `long_help_id`, `about_id`, `synopsis_id`.
- Exposure and naming:
  `cli(long = "…", short = 'x', value_name = "…", hide_in_help)`,
  `env(name = "…")`, `file(key_path = "…")`.
- Semantics: `required`, `default = "…"`,
  `deprecated(note_id = "…")`, `value(type = "duration|ipaddr|url|…")`.
- Extras:
  `example(code = "…", title_id = "…", body_id = "…")*`,
  `link(uri = "…", text_id = "…")*`, `note(text_id = "…")*`.
- App or subcommand:
  `headings(name = "…", …)`,
  `discovery(formats = […], xdg = bool, override_flag = "…",`
  `override_env = "…")`,
  `precedence(order = ["defaults", "file", "env", "cli"],`
  `rationale_id = "…")`.
- Windows (optional, generator hints):
  `windows(module_name = "…", include_common_parameters = true,`
  `split_subcommands = false)`.

### 3.3 Diagnostics

Hard errors at macro time:

- Duplicate `env.var_name` or `file.key_path` across fields emits a hard error
  with spans on both fields and a remediation hint.
- Illegal characters in environment variable names or file key paths emit a
  hard error with suggested canonical forms.
- Ambiguous value typing suggests `value(type = …)`.

Warnings:

- Unknown or unused locale IDs, missing heading overrides, or Windows hints on
  non-Windows targets.

### 3.4 Auto-ID generation

Deterministic IDs when omitted:

- App about: `"{crate}.about"`.
- Field help: `"{crate}.fields.{command_path}.{field}.help"`.
- Long help: `"{crate}.fields.{command_path}.{field}.long_help"`.
- Headings: library defaults such as `"ortho.headings.options"`.

`command_path` is `sub1.sub2` for nested subcommands.

### 3.5 Implementation notes (macro v1.1)

- `bin_name` is emitted only when `bin_name = "..."` is supplied; generators
  may fall back to `app_name` or Cargo metadata when it is `null`.
- Precedence metadata is always emitted. When `precedence(...)` is absent, the
  macro uses the default order `[defaults, file, env, cli]` with no rationale.
- Discovery metadata is emitted only when `discovery(...)` is present. The
  `search_paths` list is currently empty and should be populated by tooling
  that applies the runtime discovery rules.
- The config-path override flag is emitted only when
  `discovery(config_cli_visible = true)` is set.
- Value types are inferred for common Rust primitives; unknown types map to
  `Custom` using the final path segment (for example, `MyType`).
- Documentation-only `env(name = "...")` and `file(key_path = "...")` override
  IR output but do not affect runtime naming or loading behaviour.
- `OrthoConfig` recognizes `#[command(subcommand)]` selector fields and uses
  the field type's `OrthoConfigSubcommandDocs` implementation to populate
  recursive `subcommands`.
- Subcommand names default to kebab-cased enum variant names and honour
  `#[command(name = "...")]` / `#[clap(name = "...")]` overrides.
- Renderer regressions on populated nested trees are gated by targeted tests
  and `insta` snapshots in
  `cargo-orthohelp/tests/golden/nested_subcommand_snapshots.rs`.
- CLI metadata emits `long` only for fields that actually have a long flag.
  Positional fields emit `positional` instead; see §2.2.

### 3.6 Constructors and additive evolution

The IR types are a published contract that both the derive and hand-writing
consumers assemble. Two properties keep that contract evolvable.

**Every public IR and agent-context type is `#[non_exhaustive]`.** The sibling
modules in the same crate — `subcommand::selected`, `declarative::layer`, and
`error::types` — already take this position, and the documentation IR is the
surface most likely to grow fields as agent-native metadata lands. Without it,
adding one optional field to `FieldMetadata` breaks every struct-literal
consumer and therefore requires a major release, which in practice means the
field never gets added.

**Every type is constructible without a struct literal.** `#[non_exhaustive]`
alone would leave hand-assembling consumers with no way to build a value at
all, so each type provides a constructor taking its required arguments, with
optional metadata applied through `with_*` methods or field assignment on the
returned value:

```rust
let mut meta = DocMetadata::new("my-app", "my-app.about");
meta.bin_name = Some("my-app".into());
meta.fields.push(
    FieldMetadata::new("port", "my-app.fields.port.help")
        .with_cli(CliMetadata::flag("port").with_short('p'))
        .with_env(EnvMetadata::new("MY_APP_PORT")),
);
```

`DocMetadata::new` stamps `ir_version` from `ORTHO_DOCS_IR_VERSION`. This is
the point of the constructor rather than a convenience: a bare public version
constant that consumers read and copy into their own output lets any consumer
claim conformance to a schema version it has not implemented. Stamping the
version at construction means the value in the payload is always the version of
the library that built it. The constant remains public for comparison and
compatibility checks; it stops being the supported way to populate the field.

`HeadingIds::defaults()` returns the standard `ortho.headings.*` identifier set
so no consumer enumerates eleven identifiers by hand.

## 4. Localization model

### 4.1 Resolver

`Localizer` (trait) with `FluentLocalizer` implementation layered (consumer
bundle -> library defaults -> English). Generators pass a `&dyn Localizer` for
the target locale.

### 4.2 Catalogues

```plaintext
ortho_l10n/locales/
  en-US/messages.ftl             # canonical English library catalogue
  ja/messages.ftl                # other shipped library catalogue
consumer-package/locales/
  <locale>/<crate>.ftl           # consumer app translations (optional)
```

The library maintains its own English `en-US/messages.ftl` resource; it does
not alias an `en-GB` catalogue, and no such library resource exists. The
localizer matches language-only, so English tags including `en-GB` reuse the
embedded `en-US` resources. Additional shipped library resources use their own
locale paths, such as `ja/messages.ftl`.

PowerShell note: always emit `en-US` help XML. If generating another locale
only (for example, `en-GB`), copy it to `en-US` as a fallback because
PowerShell culture probing strongly prefers `en-US` presence.

#### 4.2.1 Default heading catalogue

`ortho_l10n` owns the complete `ortho.headings.*` catalogue, and `ortho_config`
reaches it through its runtime re-exports. A consumer that has authored no
Fluent catalogue renders correct headings from the first generator run; a
consumer that adds a locale translates the headings it wants and inherits the
rest. English locale tags such as `en-GB` use the same embedded `en-US`
resources through language-only matching.

The catalogue is not yet in `ortho_l10n/locales/en-US/messages.ftl`. That
resource holds the compatibility copy for the legacy CLI, Clap error, and
profile-error paths; the heading strings are still the hardcoded table in
`cargo-orthohelp/src/ir.rs` described below. The ownership statement above is
the intended end state of the move, which this design's §4.2 layout is the
first step towards.

The catalogue is the fallback of record. `cargo-orthohelp` currently carries a
hardcoded English table for the standard heading identifiers, which keeps raw
identifiers out of output but makes the headings the one part of a generated
man page that cannot be translated without the consumer reimplementing them.
Moving those strings into the library catalogue means the resolver layering in
§4.1 — consumer bundle, then library defaults, then English — does the work,
and the generator holds no locale content of its own.

Requiring each adopter to author eleven heading identifiers before any output
renders is pure onboarding tax, and it is identical for every adopter, which is
what makes it the library's job.

## 5. Naming and flattening

### 5.1 File key paths (dotted)

- Derived from nested field structure; segments default to snake_case.
- Override via `#[ortho_config(file(key_path = "…"))]`.
- Validate `[A-Za-z0-9_-]+` per segment (library default). Render quoting
  guidance in docs if users need non-ASCII values.

### 5.2 Environment variable names

- Prefix from crate (uppercased; non-alphanumeric -> `_`), for example,
  `my-app` -> `MY_APP`.
- Segments: top-level -> `FIELD`; nested -> `PARENT__CHILD` (double
  underscore between segments).
- Final: `{PREFIX}_{SEGMENTS}`.

Examples:

- `database.host` -> `MY_APP_DATABASE__HOST`.
- `database_host` -> `MY_APP_DATABASE_HOST`.

### 5.3 Collision detection

- Build maps of `env.var_name` and `file.key_path`. If a duplicate maps to a
  different field, raise a hard error with remediation text.

## 6. `cargo orthohelp` CLI

### 6.1 Interface (proposed)

```bash
cargo orthohelp \
  [--package <pkg>] [--bin <name> | --lib] \
  [--root-type <path::to::Type>] \
  [--locale <lang>] [--all-locales] \
  [--format ir|man|ps|agent-context|all] [--json] \
  [--out-dir <path>] \
  [--man-section <N>] [--man-date <YYYY-MM-DD>] [--man-split-subcommands] \
  [--ps-module-name <Name>] [--ps-split-subcommands <BOOL>] \
  [--ps-include-common-parameters <BOOL>] [--ps-help-info-uri <URI>] \
  [--ensure-en-us <BOOL>] [--check-agent-native] \
  [--policy-mode <off|warn|deny>] \
  [--cache] [--no-build]
```

`ir`, `man`, `ps`, `agent-context`, and `all` are the currently implemented
formats. The current default is `ir`; unsupported format values fail during
Clap parsing before generation begins. `--check-agent-native` is implemented as
the early policy pipeline stage described in §6.3.2, and generated artefacts
continue to report success or failure through process exit status.
`--policy-mode <off|warn|deny>` overrides the report mode and requires
`--check-agent-native`. `--json` remains planned for general command results
and diagnostics; the agent-native formats have dedicated stdout contracts.

The existing format behaviours are compatibility contracts until a versioned
migration is explicitly approved:

- `--format ir` writes one localized JSON file per resolved locale under
  `<out>/ir/<locale>.json`.
- `--format man` writes roff pages under
  `<out>/man/man<section>/<name>.<section>` for one locale and under
  `<out>/<locale>/man/man<section>/<name>.<section>` for multiple locales.
- `--format ps` writes a PowerShell module under
  `<out>/powershell/<ModuleName>/`, including module files, localized MAML
  help, about topics, and default `en-US` support unless `--ensure-en-us false`
  is supplied.
- `--format agent-context` writes one compact JSON document at
  `<out>/agent-context.json` and emits the same document to stdout, followed by
  a newline.
- `--format all` generates the agent-context document, IR, man pages, and
  PowerShell artefacts in a single invocation. It also emits the agent-context
  JSON to stdout and reports success or failure through process exit status.

Agent-context stdout and policy-report stdout are additive machine-readable
channels; the existing artefact paths and process success/failure contracts
remain unchanged. `--check-agent-native` emits one compact policy-report JSON
document to stdout and retains the atomic `<out>/policy-report.json` artefact.
If an explicit generator format is also requested, the policy report owns
stdout so the invocation never concatenates multiple JSON documents;
AgentContext JSON is suppressed from stdout while policy checking is enabled.
Successful checks continue into generation and retain the selected artefacts,
including `<out>/agent-context.json` for `agent-context` and `all`. A deny-mode
finding is reported before the command fails, so generator artefacts are not
created. Other formats do not emit JSON to stdout.

`Cargo.toml` defaults:

```toml
[package.metadata.ortho_config]
root_type = "my_crate::AppConfig"
locales = ["en-GB", "fr-FR"]
man_section = 1

[package.metadata.ortho_config.windows]
module_name = "MyApp"
include_common_parameters = true
split_subcommands_into_functions = false
help_info_uri = "https://example.com/help/MyApp"
```

### 6.2 Pipeline

1. Discover the package with `cargo metadata`.
2. Determine the root type from CLI or metadata. If missing, emit an error
   with remediation guidance.
3. Select the ephemeral bridge's IR cache entry under
   `target/orthohelp/<hash>/`, then create invocation-private bridge inputs:
   - Dependencies: `user_crate`, `ortho_config`.
   - `main.rs` invokes
     `<root_type as OrthoConfigDocs>::get_doc_metadata()` and serializes the
     IR JSON to stdout.
   - `cargo-orthohelp` keeps a local copy of the IR schema (mirroring
     `ortho_config::docs`) so publish checks can build against the latest
     crates.io release while the workspace evolves.
   - The generated manifest, source, copied workspace `Cargo.lock` when
     present, and executable snapshot live in an invocation-owned temporary
     directory beneath the stable IR cache entry.
   - Cargo builds into the selected package's Cargo-resolved target directory,
     reusing compiler artefacts across bridge invocations.
4. Run the bridge and capture the IR.
5. For each locale, instantiate `FluentLocalizer` and resolve IDs to strings.
6. Emit the requested outputs into `--out-dir`.
7. Summarize artefacts and exit non-zero on hard errors.

### 6.3 Caching

Cache IR at `target/orthohelp/<hash>/ir.json` keyed by the crate fingerprint,
macro version, tool version, and the workspace `Cargo.lock` hash (when present).
`--cache` reuses it when valid; `--no-build` trusts the existing IR.

Crate fingerprints hash `Cargo.toml`, `build.rs` (when present), `src/`, and
`locales/` so changes to configuration schemas or translations invalidate the
cache.

Digests are SHA-256 and are rendered as 64 lowercase hexadecimal digits by the
crate-internal `cargo-orthohelp/src/hex.rs` helper. `sha2` 0.11 returns
`hybrid_array::Array<u8, _>` from `finalize`, and that type does not implement
`core::fmt::LowerHex`, so the previous `{:x}` formatting no longer compiles.
The helper keeps the rendered form byte-for-byte identical to the earlier
output, so existing cache directories remain addressable. See the
digest-rendering section of [developers' guide](developers-guide.md) for the
helper's re-use policy.

### 6.3.1 Agent-context pipeline additions

Agent-context generation reuses the same bridge output, then transforms the
documentation-oriented metadata into the compact contract described in
[agent-native-cli-design.md](agent-native-cli-design.md). The transform must:

- preserve schema versioning separately from `DocMetadata.ir_version`, which
  remains the compatibility marker for human documentation IR;
- include populated command trees rather than only top-level fields;
- drop localized long prose while allowing a concise en-US command summary
  for command selection;
- include canonical flags, value types, enum values, defaults, and required
  inputs;
- include command semantics such as interaction mode, mutation boundaries,
  pagination, async job metadata, profile support, delivery support, feedback
  support, and output contracts when declared;
- include renderer metadata, JSON mode stream contracts, exit-code taxonomy
  metadata, skill manifest links, capability/provenance metadata, profile
  redaction metadata, delivery/feedback parser contracts, and execution-ledger
  nouns when declared;
- emit policy warnings or failures through the same validation path used by
  `--check-agent-native`.

The reusable schema types and `ORTHO_AGENT_CONTEXT_SCHEMA_VERSION` live in
`ortho_config::agent_context`. `cargo-orthohelp` owns the adapter layer only:
loading the bridge IR, applying defaults, transforming structured metadata,
writing artefacts, and reporting diagnostics.

Within `cargo-orthohelp`'s binary entrypoint, the private `GenerationContext`
only groups borrowed, run-scoped inputs for output dispatch: package selection,
bridge IR, output directory, and the optional cached en-US localizer. It may be
used by format-generation helpers in `main.rs`; it is not a reusable library
context or an owner of those values.

For the first `--format agent-context` implementation, the adapter emits an
optional `AgentCommand.summary` from the short en-US command description. It
does not emit Fluent identifiers, long help text, roff fragments, or PowerShell
help structures. The adapter emits a positional input only when
`CliMetadata.positional` is `Some`, ordering emitted positional inputs by
`CliMetadata.positional.index` and leaving `AgentInput.long` absent. A field
with CLI metadata but no flag spelling or `positional` metadata is
non-invocable configuration surface and is not emitted. The output is written
as exactly one file at `<out>/agent-context.json` and is also emitted as one
compact JSON document to stdout, followed by a newline. `--format all` includes
the same agent-context document beside IR, man pages, and PowerShell artefacts
and emits that JSON to stdout.

`AgentInput.default` is a best-effort display string, not a normative or
machine-parseable value. The generator normalizes unstable Rust token spacing
around `::` outside quoted literals before writing agent-context JSON so
proc-macro formatting changes do not churn goldens. Ordinary, byte, raw string,
and character literal contents are preserved verbatim, and lifetime syntax is
not treated as a path separator.

`--format agent-context` is the generator format and remains unchanged by
[ADR-0007](adrs/adr-0007-downstream-context-command-naming.md). The downstream
application command convention is `<tool> context --json`; the `context` name
is reserved for application surfaces, and `cargo-orthohelp` does not add a
public `context` or `agent-context` subcommand or alias. The agent-context
output must not be scraped from rendered man pages or PowerShell help.
Rendering surfaces may consume agent metadata for examples or warnings, but
they are not the source of truth for agents.

### 6.3.2 Agent-native policy pipeline stage

`--check-agent-native` runs as an early pipeline stage that does not build the
bridge crate (Decision D11). It resolves the target package and parses
`[package.metadata.ortho_config]` only, applies the `--policy-mode` override to
the report's effective mode, evaluates the resolved
`cargo_orthohelp::policy::PolicyConfig`, writes `policy-report.json` atomically
to the output directory, emits the same compact JSON report to stdout followed
by a newline, and prints a one-line summary to standard error. In `deny` mode,
deny-level findings return a policy-violation error after the report has been
written and emitted. When an explicit generator `--format` is also requested,
the policy report remains the sole JSON document on stdout, and AgentContext
JSON is suppressed. A successful check proceeds to generation and retains the
selected generator artefacts; a deny-mode finding leaves the policy report
available but fails before generator artefacts are created.

The evaluation uses the `evaluate(config, inputs) -> PolicyReport` seam; in
7.1.1 `PolicyInputs` is empty, and roadmap 7.1.2 passes the bridge IR through
that struct additively. The report embeds the configured exceptions (full shape
including `reason`) and the canonical vocabulary block. When a generator
`--format` is explicitly requested in the same invocation, a successful check
runs first and the generator pipeline follows; when it is not explicit, the
check alone writes `policy-report.json` and exits. This mirrors the
`--format agent-context` stage's placement under §6.2 without requiring
generator preconditions (`root_type`, a library target, or an `ortho_config`
dependency).

### 6.3.3 Bridge build ownership and recovery

The `target/orthohelp/<hash>/` directory remains the stable identity for cached
IR. It is not a Cargo target directory. Each uncached bridge generation creates
private inputs and a private executable run directory beneath that entry, then
builds those inputs using the selected package's Cargo-resolved target
directory. In ordinary use this is the target chosen for the selected CLI
package. In the documentation-test caller, Cargo's resolved parent target is
also reused. The bridge does not create a new dependency target for each IR
cache key.

When the selected workspace has a `Cargo.lock`, the bridge copies it into its
private generated crate before Cargo resolves dependencies. Cargo may update
that copy for the generated root package; it does not edit the selected
workspace's lockfile. Each invocation owns its generated manifest, source,
copied lockfile, and executable snapshot. Cargo compiler artefacts remain in
the target it selected, and the bridge binary runs from its private snapshot.
Cargo JSON `compiler-artifact` records provide the generated bridge executable
path; bridge code must not infer it from `target/debug` or another
target-layout assumption. The documentation integration test gets the
`cargo-orthohelp` executable from Cargo's `CARGO_BIN_EXE_cargo-orthohelp`
variable and snapshots that binary into its private run directory. It does not
rebuild the CLI binary as part of the test; only the generated bridge uses the
JSON build-and-snapshot path described here.

The generated manifest disables automatic binary discovery and names its
explicit bridge target from the selected package root and unique invocation
directory. This keeps the executable path private to that invocation, even when
an ordinary workspace target is named `orthohelp_bridge`, while preserving the
selected target directory and shared dependency cache.

A bridge-only build helper holds an operating-system file lock in the selected
Cargo target directory across the Cargo build and executable snapshot. Cargo's
own build lock ends before the bridge can copy its output. The generated binary
name includes the unique invocation directory, so an ordinary workspace build
cannot replace the reported executable during that interval. The additional
lock serializes bridge invocations sharing the selected target and protects
their marker lifecycle. This helper is private to the production bridge
pipeline. The documentation-test `BuildState` helper is test-only and shared by
the Rust-example and CLI documentation test targets; compilation and artefact
extraction stay with the Rust-example helper. Neither helper is a general
application subprocess API.

The bridge build retains Cargo's configured compiler and documentation
wrappers, wrapper settings, job limits, warning flags, and coverage
instrumentation. Metadata discovery maps `cargo llvm-cov show-env`'s
`CARGO_LLVM_COV_TARGET_DIR` value to `CARGO_TARGET_DIR`; the former is the
tool-selected target even though `show-env` does not export the latter. This
keeps nested metadata on the instrumented parent target. A nested build does
not forward the coverage CLI as `RUSTC_WRAPPER`, because that CLI cannot act as
the compiler wrapper for a second Cargo process. Coverage flags and profile
output remain enabled, and configured Cargo wrappers continue to apply. The
bridge does not sanitize Rust flags to force a separate compiler fingerprint or
change the default shared Cargo cache. The Cargo JSON reader retains at most 1
MiB of rendered compiler diagnostics and records when it truncates output. It
has no presentation sink; `run_cargo` forwards the retained diagnostics to
stderr after draining the JSON stream and emits a truncation notice when
needed. IR cache publication remains atomic and skips replacement when the
existing cache already contains identical content, preserving the established
path and idempotent-write behaviour.

The bridge cache lookup, target lock, Cargo build, executable snapshot, bridge
execution, and IR publication emit named tracing spans or events. Outcome
events use a closed operation and outcome vocabulary and include elapsed time;
they omit paths, cache keys, error strings, and child output. The one-shot CLI
installs a tracing subscriber but no metrics recorder, so this path reports
per-invocation evidence through tracing rather than emitting metrics that the
default invocation would silently discard. This boundary is recorded in
[ADR-014](adrs/adr-0014-documentation-build-ownership.md#decision-outcome).

An interrupted build whose writer lifetime cannot be confirmed leaves a
fail-closed ownership marker. A later invocation must not treat that marker as
stale automatically. Recovery requires confirming that the bridge's Cargo and
compiler processes have exited before removing it; unrelated processes and
shared cache services are outside the bridge's ownership.

## 6.4 Localized IR JSON output

`cargo-orthohelp` emits a localized IR JSON file per locale into
`<out>/ir/<locale>.json`. The schema mirrors `DocMetadata` but resolves every
Fluent identifier into a concrete string. The output includes the locale for
traceability and preserves non-localized fields such as value types or Windows
metadata.

When a Fluent ID cannot be resolved, the output uses `[missing: <id>]` as a
sentinel so generators can surface gaps during development.

### 6.4.1 Localized IR schema

```json
{
  "ir_version": "2.0",
  "locale": "en-US",
  "app_name": "my-app",
  "bin_name": "my-app",
  "about": "My App CLI",
  "synopsis": "Run the app",
  "sections": {
    "headings": {
      "name": "NAME",
      "synopsis": "SYNOPSIS",
      "description": "DESCRIPTION",
      "options": "OPTIONS",
      "environment": "ENVIRONMENT",
      "files": "FILES",
      "precedence": "PRECEDENCE",
      "exit_status": "EXIT STATUS",
      "examples": "EXAMPLES",
      "see_also": "SEE ALSO"
    }
  },
  "fields": [
    {
      "name": "port",
      "help": "Port to bind",
      "long_help": null
    }
  ],
  "subcommands": [
    {
      "app_name": "run",
      "about": "Run the app",
      "fields": []
    }
  ]
}
```

Fields that remain identifiers in the base IR are renamed to text in the
localized IR:

- `about_id` -> `about`
- `synopsis_id` -> `synopsis`
- `headings_ids` -> `headings`
- `help_id`/`long_help_id` -> `help`/`long_help`
- `note_id`/`title_id`/`text_id` -> `note`/`title`/`text`

### 6.4.2 Locale resource discovery

Consumer Fluent resources are loaded from `locales/<locale>/*.ftl` within the
target package. Files are read in lexicographic order and layered over the
embedded `ortho_config` defaults. If the requested locale does not have
embedded defaults but consumer resources exist, the generator uses the consumer
resources alone.

## 7. Output generators

### 7.1 Man page (roff)

Files: `man/man<section>/<name>.<section>` (or `…/<name>-<sub>.<section>` when
split). Use classic `man` macros: `.TH`, `.SH`, `.SS`, `.TP`, `.B`, `.I`.

Sections: NAME, SYNOPSIS, DESCRIPTION, OPTIONS, ENVIRONMENT, FILES, PRECEDENCE,
EXAMPLES, SEE ALSO, EXIT STATUS.

Rules mirror v1: CLI fields in OPTIONS; environment variables in ENVIRONMENT;
config keys and discovery in FILES; precedence explained; examples rendered
verbatim.

### 7.2 PowerShell help (MAML) and wrapper module

Artefacts (per locale):

```plaintext
<out>/powershell/<ModuleName>/
  <ModuleName>.psm1
  <ModuleName>.psd1
  <culture>/<ModuleName>-help.xml  # always include en-US
  <culture>/about_<ModuleName>.help.txt  # conceptual, optional but recommended
  completions.ps1                  # optional separate script
```

Wrapper function:

```powershell
# <ModuleName>.psm1
[CmdletBinding(PositionalBinding = $false)]
param()

function <BinName> {
  [CmdletBinding(PositionalBinding = $false)]
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
  $binRoot = if ($env:ORTHOHELP_BIN_DIR) {
    $env:ORTHOHELP_BIN_DIR
  } else {
    Join-Path $PSScriptRoot '..' 'bin'
  }
  $exe = Join-Path $binRoot '<bin>.exe'
  $exe = (Resolve-Path $exe).ProviderPath
  & $exe @Args
  $global:LASTEXITCODE = $LASTEXITCODE
}

# Completions registration
$sb = {
  param($wordToComplete, $commandAst, $cursorPosition)
  # Delegate to generated completion logic or script body
}
$hasNative = (Get-Command Register-ArgumentCompleter).Parameters.ContainsKey(
  'Native'
)
if ($hasNative) {
  Register-ArgumentCompleter -Native -CommandName '<BinName>' -ScriptBlock $sb
} else {
  Register-ArgumentCompleter -CommandName '<BinName>' -ScriptBlock $sb
}
```

Manifest (minimum):

```powershell
@{
  RootModule = '<ModuleName>.psm1'
  ModuleVersion = '0.1.0'
  CompatiblePSEditions = @('Desktop', 'Core')
  FunctionsToExport = @('<BinName>')
  # Only set if hosting Update-Help payloads.
  # HelpInfoUri = 'https://example.com/help/<ModuleName>'
  ExternalHelp = '<ModuleName>-help.xml'
}
```

MAML mapping:

- One `<command:command>` for the wrapper function. If
  `--ps-split-subcommands` is set, also export `<BinName>_<sub>` functions and
  generate separate `<command:command>` nodes.
- Parameters from `CliMetadata`:
  - Switches map to `SwitchParameter`.
  - Values map to `String`, `Int32`, `Double`, and so on inferred from
    `ValueType`.
  - Required or position heuristics: position only when unambiguous; otherwise
    named.
- Field descriptions come from `help_id` or `long_help_id`.
- Enum allowed values are appended to the description.
- Environment or file exposure is documented in Notes per parameter.
- App examples or links map to `<command:examples>` or
  `<maml:relatedLinks>`.
- Include `<CommonParameters/>` unless disabled via IR or CLI flag.

Culture folders: always generate `en-US`. Add additional cultures per
`--locale` or `--all-locales`.

Line endings and encoding: emit `.psm1` and `.psd1` with carriage return line
feed (CRLF) line endings. Emit MAML XML as UTF-8 with a byte order mark (BOM)
for maximum compatibility.

## 8. Templates and formatting

- Default `value_name` when absent: `STRING`, `INT`, `FLOAT`, `PATH`,
  `DURATION`, `IP`, `URL`, `CHOICE`, `LIST`, `MAP`.
- OPTIONS grouped by top-level `file.key_path` segment, then by flag name.
- ENVIRONMENT sorted by variable name; FILES grouped by table.
- Headings use Fluent IDs; library defaults apply when missing.

## 9. Error handling and diagnostics

Macro time:

- Duplicates, illegal names, and ambiguous types are hard errors with spans
  and remediation text.

Generation time:

- Missing Fluent messages emit warnings and fall back to English or
  `[missing: …]` in development mode.
- MAML validation errors include line and column information.
- Wrapper or function name mismatches with the MAML `<command:name>` emit an
  error.
- Missing `en-US` when other cultures exist and `--ensure-en-us` is enabled
  triggers a copy and warning.

Exit non-zero on hard errors and list artefacts on success.

## 10. Testing strategy

- Macro unit tests: attribute parsing, ID generation, collision detection.
- Roff unit tests: escaping, width wrapping, enum rendering.
- MAML unit tests: schema sanity and value type mapping.
- Golden tests: generate outputs for a fixture config across locales and
  compare against goldens.
- Windows integration tests:
  - `powershell.exe` (5.1) and `pwsh` (7+) import the generated module,
  `Get-Help {BinName} -Full` works, and CommonParameters render.
  - The argument completer registers with or without `-Native`.
  - Wrapper preserves `$LASTEXITCODE`.

## 11. Packaging and MSI guidance

- Install the executable to `C:\Program Files\<Vendor>\<Product>\bin\` and
  add that folder to PATH (machine scope).
- Install the PowerShell module to both module roots:
  - `C:\Program Files\WindowsPowerShell\Modules\<ModuleName>\`.
  - `C:\Program Files\PowerShell\Modules\<ModuleName>\`.
- Place culture subfolders (`en-US`, `en-GB`, and so on) under the module
  directory.
- Code-sign the executable and MSI; module scripts are optional but recommended
  in locked-down environments.

These are packaging recommendations; the generator writes only to `--out-dir`.
By default, generated wrappers assume the executable is in a sibling `bin`
directory relative to the module root. Set `ORTHOHELP_BIN_DIR` when packaging
installs the executable in a different location.

## 12. Versioning and compatibility

- IR: `ir_version = "2.0"`. Generated default identifiers now use mounted
  command paths rather than dotted ones, so 2.0 is a major bump for consumers
  keyed on the old identifier values; the optional `profiles` metadata rides
  the same version. `1.2` added optional `behaviour` metadata and `1.1` added
  Windows metadata, both additive minor bumps. Additive optional fields bump
  the minor version; future breaking schema changes bump the major version. All
  three versions are unreleased: `v0.9.1` still carries `"1.1"`.
- Tooling: `cargo-orthohelp` tracks the IR major.
- Runtime: `clap` v4.x unchanged; PowerShell targets 5.1+ and 7+.

Documentation IR and agent-context schemas are independently versioned sibling
contracts. Adding compact agent metadata does not require a documentation IR
major-version bump unless the existing human-documentation JSON changes in a
breaking way. New optional metadata fields must define explicit defaults for
older derives. Those defaults are applied by OrthoConfig readers, generators,
or transforms; JSON Schema annotations document the value but do not populate
it during validation.

Human-facing documentation consumers may keep reading generated roff and
PowerShell artefacts without adopting agent-context metadata. Consumers that
parse localized IR directly should tolerate additive optional fields and should
not require future agent-context or policy-report fields unless they opt into
those formats.

The policy-report schema (`cargo_orthohelp::policy`,
`ORTHO_POLICY_REPORT_SCHEMA_VERSION = "1"`) grows additively only. The
`exceptions` and `vocabulary` fields added for 7.1.1 are serde-defaulted, so
consumers that ignore unknown fields continue to work. Roadmap 7.1.2 plans
further additive fields (`PolicyInputs` bridge-IR input and a
`replacement: Option<String>` on results) without bumping the schema version.

### 12.1 Rust API compatibility

The wire contract and the Rust API evolve under separate rules. A field that is
additive on the wire is still breaking in Rust if consumers build the type with
a struct literal. The published types therefore follow §3.6:
`#[non_exhaustive]` plus constructors. With both in place, adding an optional
metadata field is a minor release for the crate as well as an additive change
to the schema.

Applying `#[non_exhaustive]` is itself a breaking change for any consumer that
currently writes struct literals. It ships together with the constructors, in
one release, with a migration note; splitting them would leave a release in
which the types cannot be constructed outwith the crate.

### 12.2 Unknown-variant tolerance

§8.2 of [agent-native-cli-design.md](agent-native-cli-design.md) permits adding
enum variants within a major version "only when the contract defines and tests
an unknown-variant fallback that preserves the documented legacy default for
strict deserializers". `InteractionMode` and `MutationEffect` each ship an
`Unknown` variant for exactly this purpose, but nothing routes unrecognized
wire strings to it, so a v1 reader hard-errors on the v2 payload it was
designed to tolerate.

Enums whose `Unknown` variant exists to absorb future values annotate that
variant with `#[serde(other)]`. Two consequences are part of the contract:

- An unrecognized value deserializes to `Unknown` and re-serializes as
  `"unknown"`. Round-tripping a payload through an older reader is lossy for
  that field. This is the intended trade: the alternative is a hard error.
- The fallback applies only to enums that document forward compatibility as
  their purpose. It is not applied to enums that model closed operator input,
  where an unrecognized value is a mistake the user should see. `PolicyMode`
  (`off`, `warn`, `deny`) is such an enum: a misspelled mode must fail loudly
  rather than silently degrade, so it keeps strict deserialization and gains no
  `Unknown` variant.

## 13. Worked example (abridged)

### 13.1 IR JSON (excerpt, 2.0)

```json
{
  "ir_version": "2.0",
  "app_name": "my-app",
  "bin_name": "my-app",
  "about_id": "my-app.about",
  "sections": {
    "headings_ids": {
      "name": "ortho.headings.name",
      "synopsis": "ortho.headings.synopsis",
      "description": "ortho.headings.description",
      "options": "ortho.headings.options",
      "environment": "ortho.headings.environment",
      "files": "ortho.headings.files",
      "precedence": "ortho.headings.precedence",
      "exit_status": "ortho.headings.exit_status",
      "examples": "ortho.headings.examples",
      "see_also": "ortho.headings.see_also"
    }
  },
  "fields": [
    {
      "name": "port",
      "help_id": "my-app.fields.port.help",
      "value": {"Integer": {"bits": 16, "signed": false}},
      "default": {"display": "8080"},
      "required": false,
      "cli": {
        "long": "port",
        "short": "p",
        "value_name": "NUM",
        "multiple": false,
        "takes_value": true,
        "possible_values": [],
        "hide_in_help": false
      },
      "env": {"var_name": "MY_APP_PORT"},
      "file": {"key_path": "port"}
    }
  ],
  "windows": {
    "module_name": "MyApp",
    "export_aliases": [],
    "include_common_parameters": true,
    "split_subcommands_into_functions": false,
    "help_info_uri": null
  },
  "subcommands": [
    {
      "ir_version": "2.0",
      "app_name": "run",
      "about_id": "run.about",
      "fields": [],
      "windows": null,
      "subcommands": [
        {
          "ir_version": "2.0",
          "app_name": "audit",
          "about_id": "audit.about",
          "fields": [],
          "windows": {
            "module_name": "MyAppAdmin",
            "export_aliases": ["my-app-audit"],
            "include_common_parameters": false,
            "split_subcommands_into_functions": true,
            "help_info_uri": null
          },
          "subcommands": []
        }
      ]
    }
  ]
}
```

### 13.2 Wrapper (psm1) excerpt

```powershell
[CmdletBinding(PositionalBinding = $false)]
param()
function my-app {
  [CmdletBinding(PositionalBinding = $false)]
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
  $exe = Join-Path $PSScriptRoot '..' 'bin' 'my-app.exe'
  $exe = (Resolve-Path $exe).ProviderPath
  & $exe @Args
  $global:LASTEXITCODE = $LASTEXITCODE
}
$sb = { param($wordToComplete, $commandAst, $cursorPosition) # … }
$hasNative = (Get-Command Register-ArgumentCompleter).Parameters.ContainsKey(
  'Native'
)
if ($hasNative) {
  Register-ArgumentCompleter -Native -CommandName 'my-app' -ScriptBlock $sb
} else {
  Register-ArgumentCompleter -CommandName 'my-app' -ScriptBlock $sb
}
```

### 13.3 Manifest (psd1) excerpt

```powershell
@{
  RootModule = 'MyApp.psm1'
  ModuleVersion = '0.1.0'
  CompatiblePSEditions = @('Desktop', 'Core')
  FunctionsToExport = @('my-app')
  ExternalHelp = 'MyApp-help.xml'
}
```

## 14. Implementation plan (delta)

1. Bump the IR schema to 1.1 and add `WindowsMetadata` plus CLI flags.
2. Make the PowerShell wrapper mandatory when `--format ps|all` is selected
   and default to generating PowerShell artefacts on Windows unless
   `--format man` is set.
3. Enforce `en-US` emission and implement `--ensure-en-us` (on by default).
4. Add `-Native` detection logic to the module template and ensure
   `$LASTEXITCODE` propagation.
5. Extend the MAML writer to emit `<CommonParameters/>` and generate the
   about topic file.
6. Add Windows integration tests (PowerShell 5.1 and 7+).
