Feature: Packaging configuration

  Scenario: goreleaser configuration
    Given the goreleaser configuration file
    When it is parsed as YAML
    Then the nfpms section exists

  Scenario: service unit hardening
    Given the systemd unit file
    Then it includes hardening directives
