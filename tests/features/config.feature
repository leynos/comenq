Feature: Daemon configuration

  Scenario: loading a valid configuration file
    Given a configuration file with token "abc"
    When the config is loaded
    Then github token is "abc"

  Scenario: missing configuration file
    Given a missing configuration file
    When the config is loaded
    Then config loading fails

  Scenario: environment variable overrides file
    Given a configuration file with token "abc"
    And environment variable "COMENQD_SOCKET_PATH" is "/tmp/env.sock"
    When the config is loaded
    Then socket path is "/tmp/env.sock"
