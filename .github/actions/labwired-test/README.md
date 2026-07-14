# Core labwired-test action

This composite action downloads a LabWired Core release archive for the GitHub
Actions runner, then runs labwired test. It is useful when a workflow needs the
archive-backed runner rather than the container image.

~~~yaml
- name: Run LabWired tests
  uses: w1ne/labwired-core/.github/actions/labwired-test@v0.18.0
  with:
    version: v0.18.0
    script: tests/firmware-test.yaml
    output-dir: out/labwired
    args: --no-uart-stdout
    upload-artifacts: 'false'
~~~

The action installs the matching archive asset named
labwired-v0.18.0-<platform>.tar.gz from w1ne/labwired-core, writes a JUnit
report plus result.json and uart.log, and fails when an assertion fails.

Inputs use this action's hyphenated names: script (required), version
(default v0.18.0), args, junit, output-dir, upload-artifacts, repo, and
github-token. The github-token input defaults to the workflow job token and
can be overridden when a workflow needs different release-download access.
