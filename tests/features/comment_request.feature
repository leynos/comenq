Feature: CommentRequest serialisation

  Scenario: serialising a valid request
    Given a default comment request
    When it is serialised
    Then the JSON is correct

  Scenario: parsing invalid JSON
    Given invalid JSON
    When it is parsed
    Then an error is returned
