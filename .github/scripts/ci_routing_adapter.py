#!/usr/bin/env python3
"""Bridge the repository routing oracle to the framework matrix discoverers."""

from __future__ import annotations

import sys
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[2]
AUDIT_ROOT = Path(__file__).resolve().parent / "repository_audit"
POLICY_ROOT = REPOSITORY / ".github" / "policy"
if str(AUDIT_ROOT) not in sys.path:
    sys.path.insert(0, str(AUDIT_ROOT))

from audit_ci_routing import RoutingError, load_routing_config, route_changes  # noqa: E402
from audit_markdown import (  # noqa: E402
    AuditError,
    load_config as load_ownership_config,
    parse_name_status_z,
    require_clean_base_checkout,
    run_git,
)


class RoutingSelectionError(RuntimeError):
    """The changed-file scope cannot be trusted for matrix selection."""


def routing_report(base: str, head: str = "HEAD") -> dict:
    """Return the strict routing report for a complete rename-aware diff."""
    if not base:
        raise RoutingSelectionError("a base revision is required for changed selection")
    try:
        require_clean_base_checkout(REPOSITORY)
        changes = parse_name_status_z(
            run_git(
                REPOSITORY,
                ["diff", "--name-status", "-z", "--find-renames", f"{base}...{head}", "--"],
            )
        )
        report = route_changes(
            REPOSITORY,
            changes,
            load_routing_config(POLICY_ROOT / "ci-routing.json"),
            load_ownership_config(POLICY_ROOT / "markdown-audit.json"),
            max_files=50_000,
            max_text_files=2_000,
        )
    except (AuditError, RoutingError, OSError, ValueError) as exc:
        raise RoutingSelectionError(str(exc)) from exc
    if report["unknown_paths"]:
        sample = ", ".join(report["unknown_paths"][:5])
        raise RoutingSelectionError(
            "unclassified non-document paths require a policy decision: " + sample
        )
    return report
