Feature: Worker task

  Scenario: successful comment posting
    Given a queued comment request
    And GitHub returns success
    When the worker runs briefly
    Then the comment is posted
    And the posting history records the success

  Scenario: API failure requeues job
    Given a queued comment request
    And GitHub returns an error
    When the worker runs briefly
    Then the queue retains the job
    And the posting history records the failure
