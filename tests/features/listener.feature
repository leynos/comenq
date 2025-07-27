Feature: Daemon listener

  Scenario: handling a valid request
    Given a running listener task
    When a client sends a valid request
    Then the request is enqueued

  Scenario: handling invalid JSON
    Given a running listener task
    When a client sends invalid JSON
    Then the request is rejected
