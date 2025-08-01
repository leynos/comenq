# Automated Cross-Platform Packaging

## Introduction

This guide provides a step-by-step process for configuring a GitHub Actions
workflow to automatically build and package the `comenq` client and `comenqd`
daemon for Linux (Fedora, Ubuntu) and macOS. macOS packaging is currently on
hold, so the workflow focuses on Linux targets only. GoReleaser manages the
entire process, from building the Rust binaries to creating platform-native
packages (`.rpm`, `.deb`) and a Homebrew formula.

The core of this process involves creating a `.goreleaser.yaml` file that
declaratively defines the build, packaging, and release steps. This file will
be used by a GitHub Actions workflow that triggers on new git tags.

### Part 1: Packaging for Fedora and Ubuntu with systemd

The first stage is to package `comenqd` as a `systemd` service for modern Linux
distributions.

#### Step 1: Create the `systemd` Unit File

First, create a `systemd` unit file that will manage the `comenqd` daemon. This
file defines how the service should be started, stopped, and managed by
`systemd`. It includes security hardening measures by specifying a dedicated
user and group and restricting filesystem access.

Create the following file in your repository. A good location is
`packaging/linux/comenqd.service`:

```systemd,ini
[Unit]
Description=Comenq Daemon
Documentation=https://github.com/leynos/comenq
After=network.target

[Service]
# Run the service as the 'comenq' user and group
User=comenq
Group=comenq

# The command to start the daemon
# Assumes the binary is installed to /usr/bin/comenqd
# The configuration file is expected at /etc/comenq/config.toml
ExecStart=/usr/bin/comenqd --config /etc/comenq/config.toml

# Security Hardening
# Disallow any privileges
CapabilityBoundingSet=
# Deny writing to the entire filesystem
ProtectSystem=strict
# Mount /home, /root, and /run/user as read-only
ProtectHome=read-only
# Use a private /tmp directory
PrivateTmp=true
# Disallow acquiring new privileges
NoNewPrivileges=true
# Restrict access to device nodes
PrivateDevices=true
# Make the kernel log, control, and audit files inaccessible
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true

# Restart the service if it fails
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

**Note:** This unit file assumes a configuration file at
`/etc/comenq/config.toml`. You should provide a default configuration file with
your package.

#### Step 2: Create a Default Configuration File

Create a default `config.toml` file to be included in the packages. Place it at
`packaging/comenqd/config.toml`.

```toml
# Default configuration for comenqd
# github_token = ""
# log_level = "info"
# socket_path = "/run/comenq/comenq.sock"
# queue_path = "/var/lib/comenq/queue"
# cooldown_period_seconds = 960
```

#### Step 3: Create the `.goreleaser.yaml` Configuration

Now, create the main GoReleaser configuration file in the root of your
repository. This file defines the entire release process.

#### `.goreleaser.yaml`

```yaml
# .goreleaser.yaml
# Visit https://goreleaser.com/customization/ for more options
project_name: comenq

# This section is only needed if you use the GoReleaser Go proxy
# to build. Since we are building a Rust project, we will override
# the build step entirely.
builds:
  - id: comenq
    binary: comenq
    main: ./crates/comenq
    goos: [linux, darwin]
    goarch: [amd64, arm64]
    # We use a custom build command with Cargo
    builder: go
    hooks:
      pre:
        - cmd: cargo build --release --package comenq --target {{ .TARGET }}
        - cmd: cp target/{{ .TARGET }}/release/comenq {{ .Path }}
  - id: comenqd
    binary: comenqd
    main: ./crates/comenqd
    goos: [linux, darwin]
    goarch: [amd64, arm64]
    # We use a custom build command with Cargo
    builder: go
    hooks:
      pre:
        - cmd: cargo build --release --package comenqd --target {{ .TARGET }}
        - cmd: cp target/{{ .TARGET }}/release/comenqd {{ .Path }}

# Create archives of the binaries.
archives:
  - id: default
    name_template: "{{ .ProjectName }}_{{ .Version }}_{{ .Os }}_{{ .Arch }}"
    format: tar.gz
    files:
      - LICENSE
      - README.md

# Configuration for creating Linux packages (.deb and .rpm)
nfpms:
  - id: comenq-packages
    package_name: comenq
    vendor: "Your Name"
    homepage: "https://github.com/leynos/comenq"
    maintainer: "Your Name <your.email@example.com>"
    description: "Client for the Comenq notification system."
    license: MIT
    formats:
      - deb
      - rpm
    # Target only the 'comenq' build
    builds:
      - comenq

  - id: comenqd-packages
    package_name: comenqd
    vendor: "Your Name"
    homepage: "https://github.com/leynos/comenq"
    maintainer: "Your Name <your.email@example.com>"
    description: "Daemon for the Comenq notification system."
    license: MIT
    formats:
      - deb
      - rpm
    # Target only the 'comenqd' build
    builds:
      - comenqd
    # Files to include in the package
    contents:
      # The systemd unit file
      - src: packaging/linux/comenqd.service
        dst: /lib/systemd/system/comenqd.service
      # The default configuration file
      - src: packaging/comenqd/config.toml
        dst: /etc/comenq/config.toml
        type: config
    # Scripts to run on installation
    scripts:
      preinstall: "packaging/linux/preinstall.sh"
      postinstall: "packaging/linux/postinstall.sh"
      preremove: "packaging/linux/preremove.sh"

# Create a GitHub release
release:
  github:
    owner: leynos
    name: comenq
  draft: true # Set to false to auto-publish

# Generate a changelog from commit messages
changelog:
  sort: asc
  filters:
    # Exclude chore, style, and test commits from the changelog
    exclude:
      - '^docs:'
      - '^test:'
      - '^chore:'
      - '^style:'
      - 'Merge pull request'
      - 'Merge branch'
```

#### Step 4: Create Installation Scripts

The `systemd` unit file requires a dedicated user. These scripts will create
the `comenq` user and group upon installation.

**packaging/linux/[preinstall.sh](http://preinstall.sh)**

```bash
#!/bin/bash
set -euo pipefail
if ! getent group comenq >/dev/null; then
    groupadd --system comenq || { echo "failed to add group" >&2; exit 1; }
fi
if ! getent passwd comenq >/dev/null; then
    useradd --system --gid comenq --home-dir /var/lib/comenq \
        --create-home --shell /sbin/nologin comenq \
        || { echo "failed to add user" >&2; exit 1; }
fi
chown comenq:comenq /var/lib/comenq
chmod 750 /var/lib/comenq
```

**packaging/linux/[postinstall.sh](http://postinstall.sh)**

```bash
#!/bin/bash
set -euo pipefail
# Reload systemd to recognize the new service, then enable and start it.
systemctl daemon-reload
systemctl enable comenqd.service
systemctl start comenqd.service
```

**packaging/linux/[preremove.sh](http://preremove.sh)**

```bash
#!/bin/bash
set -euo pipefail
# Stop and disable the service before removal.
if systemctl is-active --quiet comenqd.service; then
    systemctl stop comenqd.service
fi
if systemctl is-enabled --quiet comenqd.service; then
    systemctl disable comenqd.service
fi
```

Make these scripts executable: `chmod +x packaging/linux/*.sh`.

#### Step 5: Update the GitHub Actions Workflow

Finally, modify your existing `.github/workflows/release.yml` to use
GoReleaser. This workflow will trigger when you push a new tag (e.g., `v1.2.3`).

#### `.github/workflows/release.yml`

```yaml
name: Release

on:
  push:
    tags:
      - 'v[0-9]*.[0-9]*.[0-9]*'

jobs:
  goreleaser:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Set up Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
          targets: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
          cache: cargo

      - name: Set up Go
        uses: actions/setup-go@v5
        with:
          go-version: '1.21'

      - name: Run GoReleaser
        uses: goreleaser/goreleaser-action@v5
        with:
          distribution: goreleaser
          version: v1.24.0
          args: release --clean
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### Part 2: Extending to macOS with `launchd` and Homebrew

Now we will extend the configuration to support macOS by creating a `launchd`
service and a Homebrew Tap.

#### Step 1: Create the `launchd` Plist File

On macOS, services are managed by `launchd`. The equivalent of a `systemd` unit
file is a `.plist` file.

Create `packaging/darwin/comenqd.plist`:

```plist,xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.github.leynos.comenqd</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/comenqd</string>
        <string>--config</string>
        <string>/usr/local/etc/comenq/config.toml</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/usr/local/var/log/comenq/comenqd.log</string>
    <key>StandardErrorPath</key>
    <string>/usr/local/var/log/comenq/comenqd.err</string>
</dict>
</plist>

```

#### Step 2: Update `.goreleaser.yaml` for Homebrew

Now, add the `brews` section to your `.goreleaser.yaml` to generate a Homebrew
formula. This will create a formula in a separate repository (your "tap").

First, create a new, public GitHub repository named `homebrew-tap`.

Then, add the following section to `.goreleaser.yaml`:

```yaml
# .goreleaser.yaml (additions)

brews:
  - name: comenq
    # The GitHub repository for your Homebrew tap.
    tap:
      owner: leynos
      name: homebrew-tap
      # A token with write access to the tap repository.
      # You'll need to create a Personal Access Token (PAT) and add it
      # as a secret named HOMEBREW_TAP_TOKEN to your comenq repo.
      token: "{{ .Env.HOMEBREW_TAP_TOKEN }}"

    # The commit author
    commit_author:
      name: goreleaserbot
      email: bot@goreleaser.com

    # The homepage for the formula
    homepage: "https://github.com/leynos/comenq"
    description: "Client and Daemon for the Comenq notification system."
    license: "MIT"

    # Specify which builds to include. We will package both binaries
    # in the same formula.
    builds:
      - comenq
      - comenqd

    # Additional files to include in the formula.
    # We include the plist file for the service.
    # GoReleaser will automatically generate the service block.
    extra_files:
      - glob: ./packaging/darwin/comenqd.plist
        # This ensures the plist is renamed to match the formula name
        name_template: "{{ .ProjectName }}.plist"

    # The service block for `comenqd`
    service: |
      run [opt_bin/"comenqd", "--config", etc/"comenq/config.toml"]
      keep_alive true
      log_path var/"log/comenq/comenqd.log"
      error_log_path var/"log/comenq/comenqd.err"

    # Install instructions for the user
    install: |
      bin.install "comenq"
      bin.install "comenqd"
      (etc/"comenq").mkpath
      etc.install "config.toml" => "comenq/config.toml"
      (var/"log/comenq").mkpath
```

#### Step 3: Add the macOS Configuration File

The Homebrew formula will also install a default configuration. Add a copy for
macOS, perhaps identical to the Linux one, at `packaging/darwin/config.toml`.
Update the `brews.contents` section in `.goreleaser.yaml` to point to it if it
differs, or simply add it to the `files` section of the archive if it's
universal. For simplicity, let's assume the one at
`packaging/comenqd/config.toml` is sufficient and will be picked up by the
archive.

#### Step 4: Final `.goreleaser.yaml`

Here is the complete `.goreleaser.yaml` with both Linux and macOS
configurations:

```yaml
# .goreleaser.yaml
project_name: comenq

builds:
  - id: comenq
    binary: comenq
    main: ./crates/comenq
    goos: [linux, darwin]
    goarch: [amd64, arm64]
    builder: go
    hooks:
      pre:
        - cmd: cargo build --release --package comenq --target {{ .TARGET }}
        - cmd: cp target/{{ .TARGET }}/release/comenq {{ .Path }}
  - id: comenqd
    binary: comenqd
    main: ./crates/comenqd
    goos: [linux, darwin]
    goarch: [amd64, arm64]
    builder: go
    hooks:
      pre:
        - cmd: cargo build --release --package comenqd --target {{ .TARGET }}
        - cmd: cp target/{{ .TARGET }}/release/comenqd {{ .Path }}

archives:
  - id: default
    name_template: "{{ .ProjectName }}_{{ .Version }}_{{ .Os }}_{{ .Arch }}"
    format: tar.gz
    files:
      - LICENSE
      - README.md
      - packaging/comenqd/config.toml

nfpms:
  - id: comenq-packages
    package_name: comenq
    vendor: "Your Name"
    homepage: "https://github.com/leynos/comenq"
    maintainer: "Your Name <your.email@example.com>"
    description: "Client for the Comenq notification system."
    license: MIT
    formats: [deb, rpm]
    builds: [comenq]

  - id: comenqd-packages
    package_name: comenqd
    vendor: "Your Name"
    homepage: "https://github.com/leynos/comenq"
    maintainer: "Your Name <your.email@example.com>"
    description: "Daemon for the Comenq notification system."
    license: MIT
    formats: [deb, rpm]
    builds: [comenqd]
    contents:
      - src: packaging/linux/comenqd.service
        dst: /lib/systemd/system/comenqd.service
      - src: packaging/comenqd/config.toml
        dst: /etc/comenq/config.toml
        type: config
    scripts:
      preinstall: "packaging/linux/preinstall.sh"
      postinstall: "packaging/linux/postinstall.sh"
      preremove: "packaging/linux/preremove.sh"

brews:
  - name: comenq
    tap:
      owner: leynos
      name: homebrew-tap
      token: "{{ .Env.HOMEBREW_TAP_TOKEN }}"
    commit_author:
      name: goreleaserbot
      email: bot@goreleaser.com
    homepage: "https://github.com/leynos/comenq"
    description: "Client and Daemon for the Comenq notification system."
    license: "MIT"
    builds: [comenq, comenqd]
    service: |
      run [opt_bin/"comenqd", "--config", etc/"comenq/config.toml"]
      keep_alive true
      log_path var/"log/comenq/comenqd.log"
      error_log_path var/"log/comenq/comenqd.err"
    install: |
      bin.install "comenq"
      bin.install "comenqd"
      (etc/"comenq").mkpath
      etc.install "config.toml" => "comenq/config.toml"
      (var/"log/comenq").mkpath

release:
  github:
    owner: leynos
    name: comenq
  draft: true

changelog:
  sort: asc
  filters:
    exclude:
      - '^docs:'
      - '^test:'
      - '^chore:'
      - '^style:'
      - 'Merge pull request'
      - 'Merge branch'
```

### Final Steps and Usage

1. **Create a Personal Access Token (PAT)** for the Homebrew tap. Go to your
   GitHub Developer settings, create a new token with the `public_repo` scope,
   and add it as a repository secret named `HOMEBREW_TAP_TOKEN` in your
   `comenq` repository.

2. **Commit and Push:** Add all the new files (`.goreleaser.yaml`, service
   files, install scripts) to your repository.

3. **Tag a Release:** To trigger the workflow, create and push a new tag:

   ```bash
   git tag -a v0.1.0 -m "Release v0.1.0"
   git push origin v0.1.0
   ```

The GitHub Actions workflow will now run, build your binaries, create the
`.deb` and `.rpm` packages, upload them to a new GitHub Release, and finally,
publish the Homebrew formula to your `homebrew-tap` repository. Your users can
then install `comenq` using their native package managers.
