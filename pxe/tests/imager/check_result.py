import argparse
import json
from pathlib import Path
import re
import sys

SHA_LENGTHS = {"sha1": 40, "sha256": 64, "sha384": 96, "sha512": 128}
NAME_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
SUMMARY_HEADER = (
    "| image | variant | level | backend | boot_decision | grub_rc | seed_written | booted | efi / boot order | duration_s | verdict | detail |\n"
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n"
)


def load_catalog(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def catalog_entry(catalog, image):
    return next((entry for entry in catalog["images"] if entry["name"] == image), None)


def expectations(catalog, image, variant):
    default_uki = "uki:" + catalog["default_boot_artifacts_tag"]
    entry = catalog_entry(catalog, image)
    if entry is None:
        return dict(catalog["adhoc_expected"]) if variant in ("patched", default_uki) else {}
    block = entry.get("expected", {})
    if variant in block:
        return dict(block[variant])
    if variant == default_uki:
        return dict(block.get("uki", {}))
    return {}


def is_gated(catalog, image, variant):
    return bool(expectations(catalog, image, variant).get("gate"))


def split_csv(value):
    return [item.strip() for item in (value or "").split(",") if item.strip()]


def normalise_sha(value):
    value = (value or "").strip().lower()
    if not value:
        return ""
    algo, _, digest = value.rpartition(":")
    if not algo:
        algo = next((name for name, length in SHA_LENGTHS.items() if length == len(digest)), "")
    if SHA_LENGTHS.get(algo) != len(digest) or not re.fullmatch(r"[0-9a-f]+", digest):
        raise SystemExit(f"::error::image_sha must be <sha1|sha256|sha384|sha512>:<hex> with a matching digest length, got '{value}'")
    return f"{algo}:{digest}"


def select_images(catalog, spec):
    names = [entry["name"] for entry in catalog["images"]]
    spec = (spec or "all").strip()
    if spec == "all":
        return names
    if spec == "none":
        return []
    wanted = split_csv(spec)
    unknown = [name for name in wanted if name not in names]
    if unknown:
        raise SystemExit(f"::error::images not in the catalog: {', '.join(unknown)}")
    return [name for name in names if name in wanted]


def matrix(args):
    catalog = load_catalog(args.images_json)
    level = args.level or "both"
    if level not in ("level1", "level2", "both"):
        raise SystemExit(f"::error::level must be level1, level2 or both, got '{level}'")
    variants = split_csv(args.variants) or catalog["script_variants"]
    tag = (args.tag or "").strip() or catalog["default_boot_artifacts_tag"]
    level1, level2 = [], []
    for image in select_images(catalog, args.images):
        for variant in variants:
            level1.append({"image": image, "variant": variant, "gate": is_gated(catalog, image, variant), "url": "", "sha": ""})
        extra_tags = sorted(key[4:] for key in catalog_entry(catalog, image).get("expected", {}) if key.startswith("uki:"))
        for uki_tag in dict.fromkeys([tag] + extra_tags):
            variant = "uki:" + uki_tag
            level2.append({"image": image, "tag": uki_tag, "variant": variant, "gate": is_gated(catalog, image, variant), "url": "", "sha": ""})
    if args.adhoc_url:
        name = (args.adhoc_name or "adhoc").strip()
        if not NAME_PATTERN.match(name) or catalog_entry(catalog, name) is not None:
            raise SystemExit(f"::error::image_name '{name}' must match {NAME_PATTERN.pattern} and differ from every catalog image")
        if not re.match(r"^https?://\S+$", args.adhoc_url):
            raise SystemExit("::error::image_url must be an http(s) URL without spaces")
        sha = normalise_sha(args.adhoc_sha)
        level1.append({"image": name, "variant": "patched", "gate": is_gated(catalog, name, "patched"), "url": args.adhoc_url, "sha": sha})
        variant = "uki:" + tag
        level2.append({"image": name, "tag": tag, "variant": variant, "gate": is_gated(catalog, name, variant), "url": args.adhoc_url, "sha": sha})
    outputs = {
        "level1_matrix": json.dumps({"include": level1}, separators=(",", ":")),
        "level2_matrix": json.dumps({"include": level2}, separators=(",", ":")),
        "run_level1": str(level in ("level1", "both") and bool(level1)).lower(),
        "run_level2": str(level in ("level2", "both") and bool(level2)).lower(),
    }
    for key, value in outputs.items():
        print(f"{key}={value}")
    return 0


def actual_value(result, key):
    efi = result.get("efi") or {}
    if key == "efi_loader":
        return (efi.get("os_entry") or {}).get("loader")
    if key == "efi_network_first":
        return efi.get("network_first")
    return result.get(key)


def compare(result, expected):
    mismatches = []
    for key, wanted in expected.items():
        if key == "gate":
            continue
        actual = actual_value(result, key)
        if wanted == "nonzero":
            matched = type(actual) is int and actual != 0
        elif key == "efi_loader":
            matched = isinstance(actual, str) and actual.lower() == wanted.lower()
        else:
            matched = type(actual) is type(wanted) and actual == wanted
        if not matched:
            mismatches.append(f"{key} expected {wanted!r}, got {actual!r}")
    return mismatches


def efi_cell(result):
    if "efi" in result:
        efi = result["efi"] or {}
        loader = (efi.get("os_entry") or {}).get("loader") or "no OS entry"
        return f"{loader} network_first={efi.get('network_first')} order={','.join(efi.get('boot_order') or [])}"
    return "; ".join(result.get("boot_order_lines") or []) or "-"


def write_summary(path, cells):
    if not path:
        return
    row = "| " + " | ".join(str(cell).replace("|", "/").replace("\n", " ") for cell in cells) + " |\n"
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(SUMMARY_HEADER + row)


def check(args):
    catalog = load_catalog(args.images_json)
    expected = expectations(catalog, args.image, args.variant)
    gated = bool(expected.get("gate"))
    label = f"{args.image} {args.variant} {args.level}"
    result_path = Path(args.result)
    if not result_path.is_file():
        fatal_file = result_path.parent / "driver-fatal.txt"
        reason = fatal_file.read_text(encoding="utf-8").strip() if fatal_file.is_file() else "no driver-fatal.txt either; see the driver step log"
        write_summary(args.summary, [args.image, args.variant, args.level, "-", "-", "-", "-", "-", "-", "-", "DRIVER DIED", f"driver died before writing result.json: {reason}"])
        print(f"::{'error' if gated else 'warning'}::{label}: driver died before writing result.json: {reason}")
        return 1 if gated else 0
    try:
        result = json.loads(result_path.read_text(encoding="utf-8"))
    except ValueError as error:
        result = {"image": args.image, "variant": args.variant, "level": args.level, "harness_errors": [f"result.json is not valid JSON: {error}"]}
    problems = [f"result {field} is {result.get(field)!r}, leg is {wanted!r}" for field, wanted in (("image", args.image), ("variant", args.variant), ("level", args.level)) if result.get(field) != wanted]
    problems += [f"harness: {error}" for error in result.get("harness_errors") or []]
    infra = result.get("infra_limitation") or ""
    mismatches = compare(result, expected)
    if problems:
        verdict = "FAIL" if gated else "record-only"
    elif infra:
        verdict = "INFRA-LIMITED"
    elif not gated:
        verdict = "record-only"
    elif mismatches:
        verdict = "FAIL"
    else:
        verdict = "pass"
    detail = "; ".join(problems + ([infra] if infra else []) + mismatches) or "; ".join(result.get("notes") or [])
    write_summary(args.summary, [
        result.get("image"), result.get("variant"), result.get("level"), result.get("disk_backend"),
        result.get("boot_decision"), result.get("grub_rc"), result.get("seed_written"), result.get("booted"),
        efi_cell(result), result.get("duration_s"), verdict, detail,
    ])
    print(f"{label}: {verdict}{': ' + detail if detail else ''}")
    if verdict == "FAIL":
        for problem in problems + mismatches:
            print(f"::error::{label}: {problem}")
        return 1
    if verdict == "INFRA-LIMITED":
        print(f"::warning::{label}: infrastructure limitation, not a script verdict: {infra}")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Imager CI catalog: build the job matrix and judge one leg's result.json against images.json")
    commands = parser.add_subparsers(dest="command", required=True)
    matrix_parser = commands.add_parser("matrix")
    matrix_parser.add_argument("--images-json", required=True)
    matrix_parser.add_argument("--images", default="all")
    matrix_parser.add_argument("--level", default="both")
    matrix_parser.add_argument("--variants", default="")
    matrix_parser.add_argument("--tag", default="")
    matrix_parser.add_argument("--adhoc-url", default="")
    matrix_parser.add_argument("--adhoc-sha", default="")
    matrix_parser.add_argument("--adhoc-name", default="adhoc")
    check_parser = commands.add_parser("check")
    check_parser.add_argument("--images-json", required=True)
    check_parser.add_argument("--result", required=True)
    check_parser.add_argument("--image", required=True)
    check_parser.add_argument("--variant", required=True)
    check_parser.add_argument("--level", required=True, choices=["level1", "level2"])
    check_parser.add_argument("--summary", default="")
    args = parser.parse_args()
    return matrix(args) if args.command == "matrix" else check(args)


if __name__ == "__main__":
    sys.exit(main())
