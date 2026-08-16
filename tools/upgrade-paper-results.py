#!/usr/bin/env python3
"""Combine a frozen seven-configuration batch with its retuned supplement.

The corrected tuning pass changed only RayTraced, RayVoxel, and Mesh.  This
tool preserves the Sliced, Scattered, and Painted rows from an existing batch
and takes the three changed configurations (plus the edit protocol) from
``compare-terrain.py --supplement-only``.  It refuses unlike machines or
fixtures instead of manufacturing a mixed result.
"""

import argparse
import copy
import json
import os
import tempfile


REUSED_METHODS = {"Sliced", "Scattered", "Painted"}


def fail(message):
    raise SystemExit(message)


def load(path):
    with open(path) as source:
        return json.load(source)


def write_atomic(path, value):
    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".results-", suffix=".json",
                                             dir=directory)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, indent=2, sort_keys=False)
            output.write("\n")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def rows_by_method(run):
    grouped = {}
    for row in run.get("rows", []):
        grouped.setdefault(row["method"], []).append(row)
    return grouped


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="frozen complete batch or corrected accuracy run")
    parser.add_argument("supplement", help="protocol-v3 --supplement-only JSON")
    parser.add_argument("--out", required=True, help="upgraded JSON path")
    args = parser.parse_args()

    base = load(args.base)
    supplement = load(args.supplement)
    if (supplement.get("protocol_version", 0) < 3 or
            supplement.get("purpose") != "publication-supplement"):
        fail(f"{args.supplement}: expected a protocol-v3 publication supplement")

    identity_fields = ("label", "device", "level", "width", "height", "far",
                       "shadows", "lighting", "scenes")
    differences = [field for field in identity_fields
                   if base.get(field) != supplement.get(field)]
    if differences:
        fail("base and supplement differ in " + ", ".join(differences))
    if base.get("source", {}).get("dirty") or supplement.get("source", {}).get("dirty"):
        fail("refusing to publish a result assembled from a dirty checkout")

    publication_methods = supplement.get("publication_methods")
    if not publication_methods:
        fail("supplement does not record publication_methods")
    selected = [method["label"] for method in publication_methods]
    changed = set(selected) - REUSED_METHODS
    if {method["label"] for method in supplement.get("methods", [])} != changed:
        fail("supplement method set does not match the selected changed configurations")

    base_rows = rows_by_method(base)
    supplement_rows = rows_by_method(supplement)
    expected_cells = len(base.get("scenes", []))
    combined_rows = []
    for method in selected:
        source_rows = base_rows.get(method) if method in REUSED_METHODS else supplement_rows.get(method)
        if source_rows is None or len(source_rows) != expected_cells:
            fail(f"{method}: expected {expected_cells} rows, found "
                 f"{0 if source_rows is None else len(source_rows)}")
        combined_rows.extend(copy.deepcopy(source_rows))

    base_purpose = base.get("purpose", "publication")
    if base_purpose not in ("publication", "accuracy-only"):
        fail(f"{args.base}: unsupported purpose {base_purpose!r}")
    if base_purpose == "publication" and base.get("frames") != supplement.get("frames"):
        fail("publication base and supplement use different timed frame counts")

    result = copy.deepcopy(base)
    result.update({
        "protocol_version": 3,
        "purpose": base_purpose,
        "source": {
            "base": base.get("source"),
            "supplement": supplement.get("source"),
        },
        "methods": copy.deepcopy(publication_methods),
        "rows": combined_rows,
        "accuracy_valid": base_purpose == "accuracy-only",
        "edit_protocol": copy.deepcopy(supplement.get("edit_protocol")),
        "edit_rows": copy.deepcopy(supplement.get("edit_rows", [])),
    })
    write_atomic(args.out, result)
    print(f"wrote {args.out}: {len(combined_rows)} steady rows, "
          f"{len(result['edit_rows'])} edit rows")


if __name__ == "__main__":
    main()
