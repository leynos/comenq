Feature: Posting history

  Scenario: an empty history lists nothing
    Given an empty posting queue
    When the posting history is requested
    Then the history lists nothing

  Scenario: posting attempts are recorded in order
    Given an empty posting queue
    And the comments "First" and "Second" are queued for posting
    When the head comment is posted successfully
    And the head comment fails to post with "boom"
    And the posting history is requested
    Then the history lists 2 records
    And the history records a success then a failure with "boom"

  Scenario: the history honours a limit
    Given an empty posting queue
    And the comments "First", "Second" and "Third" are queued for posting
    When each queued comment is posted successfully
    And the posting history is requested with a limit of 2
    Then the history lists the 2 most recent records
