Feature: Client main function

  Scenario: sending a comment request
    Given a dummy daemon listening on a socket
    When the client sends the request
    Then the daemon receives the request

  Scenario: connection failure
    Given no daemon is listening on a socket
    When the client sends the request
    Then an error occurs
