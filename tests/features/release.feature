Feature: Release workflow

  Scenario: goreleaser step present
    Given the release workflow file
    When it is parsed as YAML
    Then the workflow uses goreleaser

  Scenario: triggers on version tags
    Given the release workflow file
    When it is parsed as YAML
    Then the workflow triggers on tags
