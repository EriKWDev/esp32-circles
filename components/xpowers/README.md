# XPowers shared component

[简体中文](README_ZH.md)

This directory contains the XPowers core shared by the ESP-IDF examples.
Example-specific AXP2101 initialization remains in each project's
`components/pmicpower` wrapper because rail enablement and task behavior vary
between examples.

The source was consolidated from identical copies previously stored in each
ESP-IDF example. It remains local because the repository also distributes the
matching Arduino library and no authoritative managed replacement has been
verified as equivalent for this repository's API, license, target, and hardware
requirements.

Upstream project: <https://github.com/lewisxhe/XPowersLib>

License: MIT; see the license notices in the source files and
[Third-party software](../../THIRD_PARTY.md).
