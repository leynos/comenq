Feature: Worker task

  Scenario: successful comment posting
    Given a queued comment request
    And GitHub returns success
    When the worker runs briefly
    Then the comment is posted
    And the posting history records the success

  Scenario: posts rotate through the configured tokens
    Given two queued comment requests
    And two GitHub tokens are configured
    And GitHub returns success
    When the worker drains the queue
    Then the posting history alternates between the two tokens

  Scenario: rotation resumes from the recorded history
    Given a queued comment request
    And two GitHub tokens are configured
    And the history already records a post with the second token
    And GitHub returns success
    When the worker runs briefly
    Then the comment is posted
    And the newest history record uses the first token

  Scenario: API failure requeues job
    Given a queued comment request
    And GitHub returns an error
    When the worker runs briefly
    Then the queue retains the job
    And the posting history records the failure
