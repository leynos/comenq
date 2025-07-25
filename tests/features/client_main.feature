@client_main
Feature: Client main function

  @happy_path
  Scenario: sending a comment request
    Given a dummy daemon listening on a socket
    When the client sends the request
    Then the daemon receives the request

  @unhappy_path
  Scenario: connection failure
    Given no daemon is listening on a socket
    When the client sends the request
    Then an error occurs
