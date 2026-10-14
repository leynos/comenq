Feature: Await a posted comment in the foreground
  The foreground client reports progress separately from the full comment URL.
  Waiting never changes queue scheduling or retry behaviour.

  Background:
    Given a running await daemon

  Scenario: Report the full comment URL after posting
    Given a comment queued for awaiting
    When I await the queued comment
    Then the await client is still waiting with a countdown
    When the production worker posts the comment
    Then awaiting succeeds with only the full comment URL on stdout

  Scenario: Unknown identifiers fail before a countdown
    When I await an unknown comment
    Then awaiting fails before any countdown

  Scenario: Posting failure leaves the client waiting through a full cooldown
    Given a comment queued for awaiting
    And GitHub fails the first await post
    When I await the queued comment
    When the production worker posts the comment
    Then awaiting succeeds with only the full comment URL on stdout
    And the worker retried after a full cooldown

  Scenario: Deleting an entry ends its foreground wait
    Given a comment queued for awaiting
    When I await the queued comment
    And the queued comment is deleted
    Then awaiting fails because the entry was removed

  Scenario: Interrupting the client preserves posting and permits another await
    Given a comment queued for awaiting
    When I await the queued comment
    And I interrupt the await client
    Then the comment remains queued
    When the production worker posts the comment
    And I await the same identifier again
    Then awaiting succeeds with only the full comment URL on stdout
