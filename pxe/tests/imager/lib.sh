IMAGER_TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BOOT_MARKER=IMAGER_CI_BOOT_OK
WORK_DIR=
LOG_DIR=
NOTES=()
CLEANUP_MOUNTS=()
CLEANUP_DEVICES=()
CLEANUP_PIDS=()
declare -A NBD_SERVER_PID=()
declare -A RESULT=()

log() {
	printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

die() {
	log "FATAL: $*"
	if [ -n "$LOG_DIR" ] && [ -d "$LOG_DIR" ]; then
		printf '%s\n' "$*" >"$LOG_DIR/driver-fatal.txt"
	fi
	exit 1
}

add_note() {
	NOTES+=("$*")
	log "note: $*"
}

require_root() {
	[ "$(id -u)" -eq 0 ] || die "must run as root: sudo unshare -m --propagation private bash $0 ..."
}

require_tools() {
	local tool
	for tool in "$@"; do
		command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
	done
}

enter_private_namespace() {
	local own_ns init_ns
	own_ns=$(readlink /proc/self/ns/mnt 2>/dev/null || true)
	init_ns=$(readlink /proc/1/ns/mnt 2>/dev/null || true)
	if [ -z "$own_ns" ] || [ "$own_ns" = "$init_ns" ]; then
		die "running in the host mount namespace; start with: sudo unshare -m --propagation private bash $0 ..."
	fi
	mount --make-rprivate / || die "could not make / rprivate inside the mount namespace"
	if [ -d /sys/firmware/efi/efivars ]; then
		mount_tracked -t tmpfs -o size=1m,mode=0755 imager-ci-efivars /sys/firmware/efi/efivars
		log "runner NVRAM hidden behind a private tmpfs on /sys/firmware/efi/efivars"
	fi
	log "mount namespace $own_ns is private"
}

refuse_paths_under_mnt() {
	local path resolved
	for path in "$@"; do
		[ -n "$path" ] || continue
		resolved=$(readlink -m -- "$path")
		case "$resolved" in
			/mnt|/mnt/*)
				die "$path resolves to $resolved, under /mnt, which this driver hides behind a private tmpfs; use a path outside /mnt (the workflow uses runner.temp)"
				;;
		esac
	done
}

mount_tracked() {
	local target=${*: -1}
	mount "$@" || die "mount $* failed"
	CLEANUP_MOUNTS+=("$target")
}

release_mount() {
	local target=$1
	local remaining=() entry dropped=
	if mountpoint -q -- "$target"; then
		umount -- "$target" 2>/dev/null || umount -l -- "$target" || die "cannot unmount $target"
	fi
	for ((entry = ${#CLEANUP_MOUNTS[@]} - 1; entry >= 0; entry--)); do
		if [ -z "$dropped" ] && [ "${CLEANUP_MOUNTS[entry]}" = "$target" ]; then
			dropped=1
			continue
		fi
		remaining=("${CLEANUP_MOUNTS[entry]}" "${remaining[@]}")
	done
	CLEANUP_MOUNTS=("${remaining[@]}")
}

mounts_below() {
	findmnt -lrn -o TARGET | awk -v prefix="$1/" 'index($0, prefix) == 1 { print gsub("/", "/"), $0 }' | sort -rn | cut -d' ' -f2-
}

unmount_below() {
	local target
	while IFS= read -r target; do
		[ -n "$target" ] || continue
		umount -- "$target" 2>/dev/null || umount -l -- "$target" 2>/dev/null || true
	done < <(mounts_below "$1")
}

top_mount_source() {
	findmnt -lrn -o TARGET,SOURCE | awk -v target="$1" '$1 == target { source = $2 } END { print source }'
}

safe_remove() {
	local target=$1
	local resolved
	resolved=$(readlink -m -- "$target")
	case "$resolved" in
		"$WORK_DIR"/*)
			;;
		*)
			log "refusing to remove $target: not under work dir $WORK_DIR"
			return 1
			;;
	esac
	if findmnt -lrn -o TARGET | awk -v path="$resolved" '$0 == path || index($0, path "/") == 1 { found = 1 } END { exit !found }'; then
		log "refusing to remove $resolved: something is still mounted at or below it"
		return 1
	fi
	rm -rf -- "$resolved"
}

detach_block_device() {
	local device=$1
	local name pid attempt remaining=() entry
	name=$(basename "$device")
	udevadm settle >/dev/null 2>&1 || true
	case "$name" in
		nbd*)
			qemu-nbd --disconnect "$device" >/dev/null 2>&1 || true
			pid=${NBD_SERVER_PID[$device]:-}
			for attempt in $(seq 1 150); do
				if { [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; } && [ ! -e "/sys/block/$name/pid" ]; then
					break
				fi
				sleep 0.2
			done
			if [ -e "/sys/block/$name/pid" ] || { [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; }; then
				log "warning: $device still connected after disconnect (server pid ${pid:-unknown})"
			fi
			unset "NBD_SERVER_PID[$device]"
			;;
		loop*)
			losetup -d "$device" 2>/dev/null || true
			for attempt in $(seq 1 150); do
				[ -e "/sys/block/$name/loop/backing_file" ] || break
				sleep 0.2
			done
			;;
	esac
	for entry in "${CLEANUP_DEVICES[@]}"; do
		[ "$entry" = "$device" ] || remaining+=("$entry")
	done
	CLEANUP_DEVICES=("${remaining[@]}")
	udevadm settle >/dev/null 2>&1 || true
}

cleanup_everything() {
	local index entry
	set +e
	for ((index = ${#CLEANUP_MOUNTS[@]} - 1; index >= 0; index--)); do
		entry=${CLEANUP_MOUNTS[index]}
		[ "$entry" = /mnt ] && unmount_below /mnt
		mountpoint -q -- "$entry" && { umount -- "$entry" 2>/dev/null || umount -l -- "$entry"; }
	done
	CLEANUP_MOUNTS=()
	for entry in "${CLEANUP_DEVICES[@]}"; do
		detach_block_device "$entry"
	done
	for entry in "${CLEANUP_PIDS[@]}"; do
		if kill -0 "$entry" 2>/dev/null; then
			kill "$entry" 2>/dev/null
			sleep 1
			kill -9 "$entry" 2>/dev/null
		fi
	done
	if [ -n "$WORK_DIR" ] && [ -d "$WORK_DIR/scratch" ]; then
		safe_remove "$WORK_DIR/scratch"
	fi
}

prepare_work_dir() {
	mkdir -p "$1"
	WORK_DIR=$(readlink -f -- "$1")
	case "$WORK_DIR" in
		/|/dev|/dev/*|/proc|/proc/*|/sys|/sys/*|/mnt|/mnt/*)
			die "unsafe work dir $WORK_DIR"
			;;
	esac
	case "$(readlink -m -- "$LOG_DIR")" in
		"$WORK_DIR/scratch"|"$WORK_DIR/scratch"/*)
			die "log dir $LOG_DIR is inside $WORK_DIR/scratch, which is deleted on exit"
			;;
	esac
	mkdir -p "$LOG_DIR"
	rm -f "$LOG_DIR/driver-fatal.txt"
	if [ -d "$WORK_DIR/scratch" ]; then
		safe_remove "$WORK_DIR/scratch" || die "stale $WORK_DIR/scratch cannot be removed"
	fi
	mkdir -p "$WORK_DIR/scratch"
}

install_cleanup_trap() {
	trap cleanup_everything EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM
}

nbd_ready() {
	[ -e /sys/module/nbd ] || modprobe nbd max_part=16 >/dev/null 2>&1 || return 1
	[ -e /sys/class/block/nbd0 ] && [ "$(cat /sys/module/nbd/parameters/max_part 2>/dev/null || echo 0)" -ge 16 ]
}

choose_disk_backend() {
	case "$1" in
		nbd)
			nbd_ready || die "nbd requested but the nbd module is unavailable or loaded with max_part < 16"
			echo nbd
			;;
		loop)
			echo loop
			;;
		auto)
			if nbd_ready; then echo nbd; else echo loop; fi
			;;
		*)
			die "disk backend must be auto, nbd or loop"
			;;
	esac
}

find_free_nbd() {
	local candidate
	for candidate in /sys/class/block/nbd*; do
		[ -e "$candidate/size" ] || continue
		if [ "$(cat "$candidate/size")" = 0 ] && [ ! -e "$candidate/pid" ]; then
			echo "/dev/$(basename "$candidate")"
			return 0
		fi
	done
	return 1
}

attach_block_device() {
	local file=$1 format=$2 mode=$3 backend=$4
	local device pid_file raw_source flags=()
	ATTACHED_DEVICE=
	case "$backend" in
		nbd)
			device=$(find_free_nbd) || die "no free nbd device"
			pid_file="$WORK_DIR/scratch/qemu-nbd-$(basename "$device").pid"
			if [ "$mode" = ro ]; then flags=(--read-only); else flags=(--discard=unmap --detect-zeroes=unmap); fi
			qemu-nbd --fork --pid-file="$pid_file" "${flags[@]}" --format="$format" --connect="$device" "$file" || die "qemu-nbd attach of $file failed"
			NBD_SERVER_PID[$device]=$(cat "$pid_file")
			CLEANUP_PIDS+=("${NBD_SERVER_PID[$device]}")
			;;
		loop)
			raw_source=$file
			if [ "$format" != raw ]; then
				raw_source="$WORK_DIR/scratch/$(basename "$file").raw"
				[ -f "$raw_source" ] || qemu-img convert -O raw "$file" "$raw_source" || die "qemu-img convert of $file failed"
			fi
			[ "$mode" = ro ] && flags=(--read-only)
			device=$(losetup --find --show --partscan "${flags[@]}" "$raw_source") || die "losetup of $raw_source failed"
			;;
		*)
			die "unknown disk backend $backend"
			;;
	esac
	CLEANUP_DEVICES+=("$device")
	ATTACHED_DEVICE=$device
	udevadm settle >/dev/null 2>&1 || true
	log "attached $file as $device ($backend, $mode)"
}

disk_partitions() {
	local disk=$1 attempt part
	for attempt in $(seq 1 25); do
		ls "${disk}"p[0-9]* >/dev/null 2>&1 && break
		partprobe "$disk" >/dev/null 2>&1 || true
		udevadm settle >/dev/null 2>&1 || true
		sleep 0.2
	done
	for part in "${disk}"p[0-9]*; do
		if [ -b "$part" ]; then echo "$part"; fi
	done
}

mount_readonly() {
	local device=$1 target=$2 fstype=$3
	mkdir -p "$target"
	case "$fstype" in
		xfs) mount -t xfs -o ro,norecovery "$device" "$target" 2>/dev/null ;;
		ext2|ext3|ext4) mount -t "$fstype" -o ro,noload "$device" "$target" 2>/dev/null ;;
		*) mount -o ro "$device" "$target" 2>/dev/null ;;
	esac || return 1
	CLEANUP_MOUNTS+=("$target")
}

probe_source_image() {
	local disk=$1
	local probe_dir="$WORK_DIR/scratch/probe-root"
	local part fstype number
	ROOT_UUID=
	IMAGE_GRUB_IN_ROOT=false
	PART_MANIFEST="$WORK_DIR/scratch/partitions.txt"
	: >"$PART_MANIFEST"
	for part in $(disk_partitions "$disk"); do
		number=$(cat "/sys/class/block/$(basename "$part")/partition")
		printf '%s|%s|%s|%s\n' "$number" \
			"$(blkid -p -s UUID -o value "$part" 2>/dev/null || true)" \
			"$(blkid -p -s PART_ENTRY_UUID -o value "$part" 2>/dev/null || true)" \
			"$(blkid -p -s LABEL -o value "$part" 2>/dev/null || true)" >>"$PART_MANIFEST"
		[ -z "$ROOT_UUID" ] || continue
		fstype=$(blkid -p -s TYPE -o value "$part" 2>/dev/null || true)
		case "$fstype" in
			ext2|ext3|ext4|xfs|btrfs) ;;
			*) continue ;;
		esac
		mount_readonly "$part" "$probe_dir" "$fstype" || continue
		if [ -f "$probe_dir/etc/fstab" ]; then
			ROOT_UUID=$(blkid -p -s UUID -o value "$part")
			[ -d "$probe_dir/boot/grub" ] && IMAGE_GRUB_IN_ROOT=true
			cp "$probe_dir/etc/fstab" "$LOG_DIR/image-fstab" 2>/dev/null || true
			log "root filesystem is partition $number ($fstype) UUID=$ROOT_UUID, /boot/grub in root: $IMAGE_GRUB_IN_ROOT"
		fi
		release_mount "$probe_dir"
	done
	[ -n "$ROOT_UUID" ] || die "no partition with /etc/fstab found in the image"
	cp "$PART_MANIFEST" "$LOG_DIR/image-partitions.txt"
}

inspect_imaged_root() {
	local disk=$1 root_uuid=$2
	local inspect_dir="$WORK_DIR/scratch/inspect-root"
	local part root_part= seed_dir
	SEED_WRITTEN=false
	for part in $(disk_partitions "$disk"); do
		if [ "$(blkid -p -s UUID -o value "$part" 2>/dev/null)" = "$root_uuid" ]; then
			root_part=$part
			break
		fi
	done
	[ -n "$root_part" ] || die "root UUID $root_uuid not found on $disk after imaging"
	mount_readonly "$root_part" "$inspect_dir" "$(blkid -p -s TYPE -o value "$root_part")" || die "cannot mount imaged root $root_part read-only"
	seed_dir="$inspect_dir/var/lib/cloud/seed/nocloud-net"
	if [ -s "$seed_dir/user-data" ] && [ -s "$seed_dir/meta-data" ] && [ -s "$seed_dir/network-config" ]; then
		SEED_WRITTEN=true
		cp "$seed_dir/user-data" "$LOG_DIR/imaged-user-data" 2>/dev/null || true
	fi
	cp "$inspect_dir/etc/default/grub" "$LOG_DIR/imaged-etc-default-grub" 2>/dev/null || true
	release_mount "$inspect_dir"
	log "imaged root $root_part: seed_written=$SEED_WRITTEN"
}

classify_boot_decision() {
	local log_file=$1 grub_in_root=$2 generation=$3
	local section
	section=$(tr -d '\r' <"$log_file" | sed -n '/Updating grub cfg/,$p')
	if [ -z "$section" ]; then
		echo not-reached
	elif [ "$grub_in_root" = true ]; then
		echo grub-in-root
	elif grep -q 'Resolved /boot from bootfs_uuid=' <<<"$section"; then
		echo override
	elif grep -qE 'Resolved /boot from .* to ' <<<"$section"; then
		echo fstab
	elif grep -q 'has no /boot entry; /boot stays on the root filesystem' <<<"$section"; then
		echo fstab-no-boot
	elif grep -q 'is the root filesystem; /boot stays on it' <<<"$section"; then
		echo no-fstab-p1-is-root
	elif grep -q 'in the image; defaulting /boot to ' <<<"$section"; then
		echo no-fstab-default-p1
	elif grep -q 'which cannot be resolved on' <<<"$section"; then
		if grep -q 'Assuming /boot is ' <<<"$section"; then echo unresolvable-guess; else echo unresolvable-fatal; fi
	elif grep -q 'Assuming /boot is ' <<<"$section"; then
		echo no-fstab-default-p1
	elif grep -qE 'Boot partition .* not found or is not a block device' <<<"$section"; then
		echo legacy-p1-missing
	elif [ "$generation" = legacy ]; then
		echo legacy-p1
	elif [ "$generation" = fstab ]; then
		echo boot-lookup-failed
	else
		echo no-decision-line
	fi
}

pick_free_port() {
	python3 -I -c 'import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()'
}

start_http_server() {
	local serve_dir=$1 port=$2 server_log=$3 attempt
	python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$serve_dir" >"$server_log" 2>&1 &
	CLEANUP_PIDS+=("$!")
	for attempt in $(seq 1 50); do
		curl -sf -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null && return 0
		sleep 0.2
	done
	die "http server on port $port did not come up: $(cat "$server_log")"
}

serve_image_and_seed() {
	local image=$1 serve_dir="$WORK_DIR/scratch/serve"
	mkdir -p "$serve_dir/seed"
	ln -sf "$image" "$serve_dir/$(basename "$image")"
	{
		printf '%s\n' '#cloud-config' 'runcmd:'
		printf '  - [sh, -c, "echo %s > %s"]\n' "$BOOT_MARKER" /dev/ttyS0 "$BOOT_MARKER" /dev/console
		printf '%s\n' 'power_state:' '  mode: poweroff' '  timeout: 60' '  condition: true'
	} >"$serve_dir/seed/user-data"
	printf 'instance-id: imager-ci-%s\nlocal-hostname: imager-ci\n' "$(date -u +%Y%m%d%H%M%S)" >"$serve_dir/seed/meta-data"
	printf 'version: 2\nethernets:\n  all-ethernets:\n    match:\n      name: "e*"\n    dhcp4: true\n' >"$serve_dir/seed/network-config"
	HTTP_PORT=$(pick_free_port)
	start_http_server "$serve_dir" "$HTTP_PORT" "$LOG_DIR/http-server.log"
	IMAGE_SHA=$(sha256sum -- "$image" | awk '{ print $1 }')
	log "serving $(basename "$image") and the seed on 127.0.0.1:$HTTP_PORT"
}

firmware_file() {
	local candidate
	for candidate in "$@"; do
		[ -f "$candidate" ] && { echo "$candidate"; return 0; }
	done
	return 1
}

ovmf_code() {
	firmware_file /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd || die "OVMF code firmware not found"
}

ovmf_vars_template() {
	firmware_file /usr/share/OVMF/OVMF_VARS_4M.fd /usr/share/OVMF/OVMF_VARS.fd || die "OVMF vars template not found"
}

qemu_accel_args() {
	if [ -w /dev/kvm ]; then echo "-machine q35,accel=kvm -cpu host"; else echo "-machine q35,accel=tcg -cpu max"; fi
}

default_timeout() {
	if [ -w /dev/kvm ]; then echo "$1"; else echo "$2"; fi
}

boot_imaged_disk() {
	local raw_disk=$1 serial_log=$2 timeout_seconds=$3 vars_file=$4 nic=$5
	local accel_args qemu_rc=0 net_args=(-nic none)
	BOOTED=false
	[ -f "$vars_file" ] || cp "$(ovmf_vars_template)" "$vars_file"
	[ "$nic" = user ] && net_args=(-netdev user,id=n0 -device virtio-net-pci,netdev=n0)
	accel_args=$(qemu_accel_args)
	log "booting imaged disk $raw_disk (timeout ${timeout_seconds}s, nic $nic, $accel_args)"
	: >"$serial_log"
	timeout "$timeout_seconds" qemu-system-x86_64 \
		$accel_args -smp 2 -m 2048 \
		-display none -monitor none -no-reboot \
		-drive if=pflash,format=raw,readonly=on,file="$(ovmf_code)" \
		-drive if=pflash,format=raw,file="$vars_file" \
		-drive file="$raw_disk",format=raw,if=none,id=target,discard=unmap,detect-zeroes=unmap \
		-device nvme,drive=target,serial=imagerci \
		"${net_args[@]}" \
		-serial file:"$serial_log" \
		>"$LOG_DIR/qemu-boot.stdout" 2>&1 &
	CLEANUP_PIDS+=("$!")
	wait "$!" || qemu_rc=$?
	if grep -q "$BOOT_MARKER" "$serial_log"; then
		BOOTED=true
		log "boot marker seen (qemu rc $qemu_rc)"
	elif [ "$qemu_rc" -eq 124 ]; then
		add_note "boot test timed out after ${timeout_seconds}s without the marker"
	else
		add_note "boot test qemu exited rc=$qemu_rc without the marker"
	fi
}

result_str() {
	RESULT[$1]=$(jq -cn --arg value "$2" '$value')
}

result_json() {
	RESULT[$1]=$2
}

write_result() {
	local path=$1 key args=()
	result_json notes "$(printf '%s\n' "${NOTES[@]}" | jq -Rsc 'split("\n") | map(select(length > 0))')"
	for key in "${!RESULT[@]}"; do
		args+=(--arg "$key" "${RESULT[$key]}")
	done
	mkdir -p "$(dirname "$path")"
	jq -nS "${args[@]}" '$ARGS.named | map_values(fromjson)' >"$path" || die "could not write $path"
	log "result written to $path"
	cat "$path" >&2
}
