Feature: CLI argument parsing

  Scenario: parsing valid arguments
    Given valid CLI arguments
    When they are parsed
    Then parsing succeeds
    And the socket path is "/run/comenq/socket"

  Scenario: overriding the socket path
    Given valid CLI arguments
    And socket path "/tmp/test.sock"
    When they are parsed
    Then parsing succeeds
    And the socket path is "/tmp/test.sock"

  Scenario: missing required arguments
    Given no CLI arguments
    When they are parsed
    Then an error is returned

  Scenario Outline: invalid repository slug
    Given CLI arguments with repo slug "<slug>"
    When they are parsed
    Then an error is returned

    Examples:
      | slug              |
      | octocat           |
      | /repo             |
      | owner/            |
      | owner/repo/extra  |
