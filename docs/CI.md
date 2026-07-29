# Continuous integration

All example compilation for this restructure is performed by GitHub Actions.

## Workflows

| Workflow | Scope | Matrix |
| --- | --- | --- |
| `esp-idf-examples.yml` | Nine first-party ESP-IDF projects | ESP-IDF v5.5.5 and v6.0.2 |
| `arduino-examples.yml` | Nine first-party Arduino sketches | Arduino-ESP32 3.3.11 |

Pull requests build only affected examples unless a shared component, library,
build script, configuration, or workflow changes. Manual dispatch accepts
`all` or an individual example directory name.

## Artifacts

Successful matrix jobs upload ZIP files with:

- binaries required by `write_flash`;
- flash offsets in `manifest.json`;
- the exact framework version and source commit;
- `flash.sh` and `flash.bat` helpers;
- ESP-IDF `flasher_args.json`, when applicable.

The packaged command still requires a compatible Python environment with
`esptool.py` and a correctly connected device.

## Version policy

Exact versions are pinned in workflow files for reproducibility. Version updates
should be made in one focused pull request after checking official stable
releases and migration notes. Do not silently reinterpret a moving `latest`
label as validated.
