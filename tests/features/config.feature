@serial
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

  Scenario: invalid TOML syntax
    Given an invalid configuration file
    When the config is loaded
    Then config loading fails

  Scenario: missing required field
    Given a configuration file without github_token
    When the config is loaded
    Then config loading fails

  Scenario: uses default socket path without a user runtime directory
    Given a configuration file with token "abc" and no socket_path
    And environment variable "XDG_RUNTIME_DIR" is unset
    When the config is loaded
    Then socket path is "/run/comenq/comenq.sock"

  Scenario: derives default socket path from the user runtime directory
    Given a configuration file with token "abc" and no socket_path
    And environment variable "XDG_RUNTIME_DIR" is "/run/user/1000"
    When the config is loaded
    Then socket path is "/run/user/1000/comenq/comenq.sock"

  Scenario: reading the token from a file
    Given a configuration file referencing a token file containing "s3cret"
    When the config is loaded
    Then github token is "s3cret"

  Scenario: token file missing on disk
    Given a configuration file referencing a missing token file
    When the config is loaded
    Then config loading fails

  Scenario: uses default cooldown period
    Given a configuration file with token "abc" and no cooldown_period_seconds
    When the config is loaded
    Then cooldown_period_seconds is 960
