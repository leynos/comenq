Feature: Queue management

  Scenario: putting a comment reports its identifier and ETA
    Given an empty comment queue
    When the comment "First comment" is put
    Then the reply carries an eight character identifier
    And the reply reports an immediate ETA

  Scenario: put identifiers reappear in listings
    Given an empty comment queue
    When the comment "First comment" is put
    Then listing shows the same identifier as the put reply

  Scenario: listing shows the pending schedule in posting order
    Given an empty comment queue
    And the comments "First", "Second" and "Third" are queued
    Then listing shows 3 entries with strictly increasing ETAs

  Scenario: listings collapse long comments to one line
    Given an empty comment queue
    When a comment of 100 "x" characters is put
    Then the listed body summarizes to one line of 60 characters

  Scenario: bumping moves a comment to the head of the queue
    Given an empty comment queue
    And the comments "First", "Second" and "Third" are queued
    When the comment "Third" is bumped
    Then the queue order is "Third", "First", "Second"

  Scenario: busting moves a comment to the tail of the queue
    Given an empty comment queue
    And the comments "First", "Second" and "Third" are queued
    When the comment "First" is busted
    Then the queue order is "Second", "Third", "First"

  Scenario: deleting removes a comment from the queue
    Given an empty comment queue
    And the comments "First", "Second" and "Third" are queued
    When the comment "Second" is deleted
    Then the queue order is "First", "Third"

  Scenario: operations on unknown identifiers are rejected
    Given an empty comment queue
    And the comments "First", "Second" and "Third" are queued
    When the unknown identifier "deadbeef" is bumped
    Then the daemon reports an unknown identifier error
