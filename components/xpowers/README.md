# XPowers shared component

This directory contains the XPowers core shared by the ESP-IDF examples.
Example-specific AXP2101 initialization remains in each project's
`components/pmicpower` wrapper because rail enablement and task behavior vary
between examples.

The source was consolidated from the identical copies previously stored in
each ESP-IDF example. It remains local because the repository also distributes
the matching Arduino library, and the available registry package is a
third-party fork rather than a Waveshare or Espressif managed component.

Upstream project: <https://github.com/lewisxhe/XPowersLib>

License: MIT; see the license notices in the source files.
