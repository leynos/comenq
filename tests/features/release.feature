Feature: Release workflow

  Scenario: shared release actions present
    Given the release workflow file
    When it is parsed as YAML
    Then the workflow uses the shared release actions

  Scenario: triggers on version tags
    Given the release workflow file
    When it is parsed as YAML
    Then the workflow triggers on tags
