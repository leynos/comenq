Feature: CommentRequest serialisation

  Scenario: serialising a valid request
    Given a default comment request
    When it is serialised
    Then the JSON is correct

  Scenario: parsing invalid JSON
    Given invalid JSON
    When it is parsed
    Then an error is returned

  Scenario: parsing JSON missing the owner field
    Given valid JSON missing the 'owner' field
    When it is parsed
    Then an error is returned

  Scenario: parsing JSON missing the repo field
    Given valid JSON missing the 'repo' field
    When it is parsed
    Then an error is returned

  Scenario: parsing JSON missing all required fields
    Given valid JSON missing all required fields
    When it is parsed
    Then an error is returned
