#!/usr/bin/env python3
import json
import os
import re
import shutil
import subprocess
import sys

PROG = "efibootmgr"
NETWORK_PATH = "PciRoot(0x0)/Pci(0x2,0x0)/MAC(525400c1c1c1,1)/IPv4(0.0.0.0,0,DHCP,0.0.0.0,0.0.0.0,0.0.0.0)"
INITIAL_STATE = {
    "boot_current": "0000",
    "timeout": 0,
    "boot_order": ["0002", "0000", "0001"],
    "entries": {
        "0000": {"label": "UEFI PXEv4 (MAC:525400C1C1C1)", "path": NETWORK_PATH, "kind": "network"},
        "0001": {"label": "UEFI HTTPv4 (MAC:525400C1C1C1)", "path": NETWORK_PATH + "/Uri()", "kind": "network"},
        "0002": {"label": "UEFI Misc Device", "path": "PciRoot(0x0)/Pci(0x3,0x0)", "kind": "other"},
    },
    "calls": [],
}
VALUE_OPTIONS = {
    "-L": "label", "--label": "label",
    "-d": "disk", "--disk": "disk",
    "-p": "part", "--part": "part",
    "-l": "loader", "--loader": "loader",
    "-o": "bootorder", "--bootorder": "bootorder",
}
FLAG_OPTIONS = {
    "-v": "verbose", "--verbose": "verbose",
    "-c": "create", "--create": "create",
    "-B": "delete", "--delete-bootnum": "delete",
}


class Failure(Exception):
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


def state_path():
    path = os.environ.get("FAKE_EFI_STATE")
    if not path:
        raise Failure(2, f"{PROG}: FAKE_EFI_STATE is not set")
    return path


def load_state(path):
    if not os.path.exists(path):
        return json.loads(json.dumps(INITIAL_STATE))
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def save_state(path, state):
    temporary = path + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(state, handle, indent=2, sort_keys=True)
    os.replace(temporary, path)


def parse_args(argv):
    options = {"verbose": 0, "create": False, "delete": False}
    index = 0
    while index < len(argv):
        arg = argv[index]
        name, inline = (arg.split("=", 1) + [None])[:2] if arg.startswith("--") else (arg, None)
        if name in FLAG_OPTIONS and inline is None:
            key = FLAG_OPTIONS[name]
            options[key] = options[key] + 1 if key == "verbose" else True
        elif name in VALUE_OPTIONS:
            if inline is None:
                index += 1
                if index >= len(argv):
                    raise Failure(2, f"{PROG}: option '{arg}' requires an argument")
                inline = argv[index]
            options[VALUE_OPTIONS[name]] = inline
        else:
            raise Failure(2, f"{PROG}: unrecognized option '{arg}' (not simulated by the imager CI fake)")
        index += 1
    actions = [key for key in ("create", "delete", "bootorder") if options.get(key)]
    if len(actions) > 1:
        raise Failure(2, f"{PROG}: combining {' and '.join(actions)} is not simulated by the imager CI fake")
    if options["delete"] and "label" not in options:
        raise Failure(2, f"{PROG}: -B is only simulated together with -L <label>")
    return options


def hd_node(disk, part):
    guid, start, size = "00000000-0000-0000-0000-000000000000", 0, 0
    if shutil.which("sgdisk"):
        result = subprocess.run(["sgdisk", "-i", str(part), disk], capture_output=True, text=True, check=False)
        found = re.search(r"Partition unique GUID: ([0-9A-Fa-f-]+)", result.stdout)
        first = re.search(r"First sector: (\d+)", result.stdout)
        last = re.search(r"Last sector: (\d+)", result.stdout)
        if found and first and last:
            guid, start, size = found.group(1).lower(), int(first.group(1)), int(last.group(1)) - int(first.group(1)) + 1
    return f"HD({part},GPT,{guid},{start:#x},{size:#x})"


def create_entry(state, options):
    disk = options.get("disk", "/dev/sda")
    part = options.get("part", "1")
    if not part.isdigit() or int(part) < 1:
        raise Failure(1, f"{PROG}: invalid partition number '{part}'")
    if not os.path.exists(disk):
        raise Failure(5, f"{PROG}: Could not prepare Boot variable: No such file or directory")
    loader = options.get("loader", "\\EFI\\ubuntu\\grub.efi")
    label = options.get("label", "Linux")
    number = next(f"{value:04X}" for value in range(0x10000) if f"{value:04X}" not in state["entries"])
    state["entries"][number] = {
        "label": label,
        "path": f"{hd_node(disk, int(part))}/File({loader})",
        "kind": "os",
        "disk": disk,
        "part": int(part),
        "loader": loader,
    }
    state["boot_order"] = [number] + [entry for entry in state["boot_order"] if entry != number]


def delete_by_label(state, label):
    doomed = [number for number, entry in state["entries"].items() if entry["label"] == label]
    if not doomed:
        raise Failure(15, f"{PROG}: Could not delete variable: No such file or directory")
    for number in doomed:
        del state["entries"][number]
    state["boot_order"] = [entry for entry in state["boot_order"] if entry not in doomed]


def set_boot_order(state, csv):
    order = []
    for token in csv.split(","):
        if not re.fullmatch(r"[0-9A-Fa-f]{1,4}", token):
            raise Failure(8, f"{PROG}: Invalid BootOrder order entry value '{token}'")
        number = f"{int(token, 16):04X}"
        if number not in state["entries"]:
            raise Failure(8, f"{PROG}: Could not set BootOrder: Boot{number} does not exist")
        order.append(number)
    state["boot_order"] = order


def listing(state, verbose):
    lines = [
        f"BootCurrent: {state['boot_current']}",
        f"Timeout: {state['timeout']} seconds",
        "BootOrder: " + ",".join(state["boot_order"]),
    ]
    for number in sorted(state["entries"]):
        entry = state["entries"][number]
        lines.append(f"Boot{number}* {entry['label']}\t{entry['path']}")
        if verbose:
            lines.append("      dp: " + " ".join(f"{byte:02x}" for byte in entry["path"].encode()[:24]))
    return "\n".join(lines) + "\n"


def run(argv):
    path = state_path()
    state = load_state(path)
    status = 0
    try:
        options = parse_args(argv)
        if options["create"]:
            create_entry(state, options)
        elif options["delete"]:
            delete_by_label(state, options["label"])
        elif "bootorder" in options:
            set_boot_order(state, options["bootorder"])
        sys.stdout.write(listing(state, options["verbose"]))
    except Failure as failure:
        sys.stderr.write(failure.message + "\n")
        status = failure.status
    state["calls"].append({"argv": argv, "status": status})
    save_state(path, state)
    return status


def report():
    state = load_state(state_path())
    order = state["boot_order"]
    created = [number for number in order if state["entries"].get(number, {}).get("kind") == "os"]
    os_entry = None
    network_first = False
    if created:
        number = created[0]
        entry = state["entries"][number]
        os_entry = {"num": number, "label": entry["label"], "loader": entry["loader"], "part": entry["part"], "disk": entry["disk"]}
        position = order.index(number)
        kinds = [state["entries"][item]["kind"] for item in order]
        network_first = all(
            (index < position) == (kind == "network")
            for index, kind in enumerate(kinds)
            if index != position
        )
    entries = [dict(state["entries"][number], num=number) for number in sorted(state["entries"])]
    json.dump(
        {"entries": entries, "boot_order": order, "os_entry": os_entry, "network_first": network_first, "calls": state["calls"]},
        sys.stdout,
        sort_keys=True,
    )
    sys.stdout.write("\n")
    return 0


def main():
    try:
        if os.environ.get("FAKE_EFI_MODE") == "report":
            return report()
        return run(sys.argv[1:])
    except Failure as failure:
        sys.stderr.write(failure.message + "\n")
        return failure.status


if __name__ == "__main__":
    sys.exit(main())
