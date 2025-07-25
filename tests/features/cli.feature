Feature: CLI argument parsing

  Scenario: parsing valid arguments
    Given valid CLI arguments
    When they are parsed
    Then parsing succeeds

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
