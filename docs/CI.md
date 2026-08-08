# Continuous integration

[简体中文](CI_ZH.md)

All example compilation for this repository is performed by GitHub Actions.
Local static policy tests are useful during development, but only a workflow run
for the committed pull-request head is build evidence.

## Workflows

| Workflow | Scope | Exact matrix |
| --- | --- | --- |
| `esp-idf-examples.yml` | Nine first-party ESP-IDF projects | ESP-IDF v5.5.5 and v6.0.2 |
| `arduino-examples.yml` | Nine first-party Arduino sketches | Arduino-ESP32 3.3.11 |
| `repository-policy.yml` | Repository policy and static tests | Python standard-library checks |

Every pull request and every push to `main` enters the workflows. A cheap
discovery job classifies the complete rename-aware base-to-head diff before any
matrix is created:

- first-party example changes select only the affected example;
- shared components, libraries, build scripts, or framework workflows select
  every example for the affected framework;
- documentation-only and preserved firmware changes select no example build;
- deleted or renamed example paths select all remaining examples for the
  affected framework;
- an empty, incomplete, or unknown change scope fails closed instead of silently
  reporting an unvalidated build result.

Each framework workflow has an always-running result job. This gives branch
protection one stable check whether the matrix ran, was intentionally skipped,
or failed during discovery. Manual dispatch accepts `changed`, `all`, or one
current example directory name.

## Repository policy

The policy workflow runs unit tests and verifies:

- English/Simplified Chinese first-party Markdown pairs and reciprocal links;
- local links, anchors, homepage structure, and public-text privacy;
- docs-only scope and rename-aware build routing;
- repository inventory and example discovery expectations;
- trailing whitespace and patch formatting through `git diff --check`.

Embedded upstream snapshots are classified explicitly and are not rewritten to
meet first-party documentation style.

## Artifacts

Successful matrix jobs upload ZIP files with:

- every binary required by `write_flash`;
- normalized flash offsets in `manifest.json`;
- the exact framework version and source commit;
- `flash.sh` and `flash.bat` helpers;
- portable ESP-IDF `flasher_args.json`, when applicable.

Packaging rejects paths outside the selected build directory, ambiguous binary
matches, duplicate normalized flash offsets or destination names, and filenames
that cannot be represented safely by both helper scripts. The packaged command
still requires a compatible Python environment with `esptool.py` and a correctly
connected device.

## Version policy

Exact stable versions are pinned in workflow files for reproducibility. Version
updates belong in a focused pull request after checking official release notes
and migration guidance. Do not reinterpret a moving `latest` label as validated,
and do not claim a version until the committed matrix for that exact version is
green.
