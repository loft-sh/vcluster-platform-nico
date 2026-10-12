#!/bin/bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SCRIPT_UNDER_TEST=
VARIANT=
IMAGE_NAME=
IMAGE_FILE=
RESULT_PATH=
WORK_DIR_REQUEST=
BOOT_TEST=1
BOOT_TIMEOUT=
DISK_BACKEND_REQUEST=auto
TARGET_SIZE=16G
ALIAS_NAME=nvme0n1

usage() {
	cat >&2 <<EOF
usage: sudo unshare -m --propagation private bash $0 --script disk_imaging.sh --variant NAME --image NAME --image-file FILE --work-dir DIR --result result.json [options]
  --log-dir DIR          where logs land (default: directory of --result)
  --boot-test 0|1        boot the imaged disk in QEMU and wait for the cloud-init marker (default 1)
  --boot-timeout SECS    QEMU boot timeout (default 600 with KVM, 1500 without)
  --disk-backend MODE    auto|nbd|loop block device behind the target disk (default auto, nbd preferred)
  --target-size SIZE     sparse target disk size (default $TARGET_SIZE)
EOF
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--script) SCRIPT_UNDER_TEST=$2; shift 2 ;;
		--variant) VARIANT=$2; shift 2 ;;
		--image) IMAGE_NAME=$2; shift 2 ;;
		--image-file) IMAGE_FILE=$2; shift 2 ;;
		--work-dir) WORK_DIR_REQUEST=$2; shift 2 ;;
		--result) RESULT_PATH=$2; shift 2 ;;
		--log-dir) LOG_DIR=$2; shift 2 ;;
		--boot-test) BOOT_TEST=$2; shift 2 ;;
		--boot-timeout) BOOT_TIMEOUT=$2; shift 2 ;;
		--disk-backend) DISK_BACKEND_REQUEST=$2; shift 2 ;;
		--target-size) TARGET_SIZE=$2; shift 2 ;;
		*) usage ;;
	esac
done

[ -n "$SCRIPT_UNDER_TEST" ] && [ -n "$VARIANT" ] && [ -n "$IMAGE_NAME" ] && [ -n "$IMAGE_FILE" ] && [ -n "$WORK_DIR_REQUEST" ] && [ -n "$RESULT_PATH" ] || usage
[ -n "$LOG_DIR" ] || LOG_DIR=$(dirname "$RESULT_PATH")
refuse_paths_under_mnt "$SCRIPT_UNDER_TEST" "$IMAGE_FILE" "$WORK_DIR_REQUEST" "$RESULT_PATH" "$LOG_DIR"
mkdir -p "$LOG_DIR"
[ -f "$SCRIPT_UNDER_TEST" ] || die "script not found: $SCRIPT_UNDER_TEST"
[ -f "$IMAGE_FILE" ] || die "image not found: $IMAGE_FILE"

require_root
install_cleanup_trap
enter_private_namespace
require_tools qemu-img qemu-nbd qemu-system-x86_64 sgdisk partprobe growpart blkid lsblk losetup findmnt mountpoint mknod stat jq python3 curl chroot timeout udevadm iconv pvscan
START_SECONDS=$SECONDS
prepare_work_dir "$WORK_DIR_REQUEST"
IMAGE_FILE=$(readlink -f -- "$IMAGE_FILE")
SCRIPT_LOG="$LOG_DIR/script-output.log"
GRUB_RC_FILE="$WORK_DIR/scratch/grub_rc"
MAIN_RC_FILE="$WORK_DIR/scratch/main_rc"
SEED_CALL_FILE="$WORK_DIR/scratch/add_cloud_init_called"
DOWNLOAD_DIR="$WORK_DIR/scratch/download"
TARGET_RAW="$WORK_DIR/scratch/target.raw"
SOURCEABLE_SCRIPT="$WORK_DIR/scratch/disk_imaging.sourceable.sh"
FAKE_BIN="$WORK_DIR/scratch/bin"
FAKE_EFI_STATE="$WORK_DIR/scratch/efi-state.json"
mkdir -p "$DOWNLOAD_DIR" "$FAKE_BIN"
BOOT_TIMEOUT=${BOOT_TIMEOUT:-$(default_timeout 600 1500)}
[ -w /dev/kvm ] || add_note "no /dev/kvm, QEMU runs with TCG"

if grep -q 'BASH_SOURCE\[0\]' "$SCRIPT_UNDER_TEST"; then
	cp "$SCRIPT_UNDER_TEST" "$SOURCEABLE_SCRIPT"
else
	sed -e '/^main$/d' -e '/^echo "Rebooting"/d' -e '/^systemctl reboot/d' "$SCRIPT_UNDER_TEST" >"$SOURCEABLE_SCRIPT"
	add_note "variant has an unguarded main; trailing main/reboot lines stripped before sourcing"
fi
bash -n "$SOURCEABLE_SCRIPT" || die "script under test does not parse"
GENERATION=legacy
grep -q 'function resolve_boot_partition' "$SOURCEABLE_SCRIPT" && GENERATION=fstab

DISK_BACKEND=$(choose_disk_backend "$DISK_BACKEND_REQUEST")
[ "$DISK_BACKEND" = nbd ] || add_note "nbd unavailable or not requested: loop backend, whose partitions cannot be aliased and whose lsblk TYPE is loop"

IMAGE_FORMAT=$(qemu-img info --output=json "$IMAGE_FILE" | jq -r '.format')
attach_block_device "$IMAGE_FILE" "$IMAGE_FORMAT" ro "$DISK_BACKEND"
PROBE_DEV=$ATTACHED_DEVICE
probe_source_image "$PROBE_DEV"
detach_block_device "$PROBE_DEV"

truncate -s "$TARGET_SIZE" "$TARGET_RAW"
attach_block_device "$TARGET_RAW" raw rw "$DISK_BACKEND"
TARGET_DEV=$ATTACHED_DEVICE

serve_image_and_seed "$IMAGE_FILE"
KERNEL_CMDLINE="BOOT_IMAGE=qcow-imager.efi loglevel=7 console=tty0 console=ttyS0,115200 pci=realloc=off image_url=http://127.0.0.1:$HTTP_PORT/$(basename "$IMAGE_FILE") image_sha=$IMAGE_SHA rootfs_uuid=$ROOT_UUID image_disk=/dev/$ALIAS_NAME ds=nocloud-net;s=http://127.0.0.1:$HTTP_PORT/seed"
printf '%s\n' "$KERNEL_CMDLINE" | tee "$WORK_DIR/scratch/cmdline" >"$LOG_DIR/kernel-cmdline"
mount_tracked --bind "$WORK_DIR/scratch/cmdline" /proc/cmdline
mount_tracked -t tmpfs -o size=16m imager-ci-mnt /mnt

printf '#!/bin/sh\nexec python3 -I '"'"'%s'"'"' "$@"\n' "$IMAGER_TEST_DIR/fake_efibootmgr.py" >"$FAKE_BIN/efibootmgr"
chmod 0755 "$FAKE_BIN/efibootmgr"

clone_char_node() {
	local source=$1 target=$2 hex_major hex_minor mode
	[ -c "$source" ] || return 0
	read -r hex_major hex_minor mode < <(stat -c '%t %T %a' "$source")
	mknod -m "$mode" "$target" c "$((16#$hex_major))" "$((16#$hex_minor))"
}

link_alias() {
	local dir=$1 value=$2 partition=$3
	[ -n "$value" ] || return 0
	value=${value//\//\\x2f}
	ln -sfn "../../$partition" "$dir/${value// /\\x20}"
}

build_private_dev() {
	local dev_root="$WORK_DIR/scratch/dev" name major minor range part node number uuid partuuid label
	name=$(basename "$TARGET_DEV")
	mkdir -p "$dev_root"
	mount_tracked -t tmpfs -o mode=0755,size=4m imager-ci-dev "$dev_root"
	for node in null zero full random urandom tty mapper/control; do
		mkdir -p "$(dirname "$dev_root/$node")"
		clone_char_node "/dev/$node" "$dev_root/$node"
	done
	mkdir -p "$dev_root/pts" "$dev_root/disk/by-uuid" "$dev_root/disk/by-partuuid" "$dev_root/disk/by-label"
	mkdir -m 1777 "$dev_root/shm"
	ln -s /proc/self/fd "$dev_root/fd"
	ln -s /proc/self/fd/0 "$dev_root/stdin"
	ln -s /proc/self/fd/1 "$dev_root/stdout"
	ln -s /proc/self/fd/2 "$dev_root/stderr"
	IFS=: read -r major minor <"/sys/class/block/$name/dev"
	mknod -m 0660 "$dev_root/$ALIAS_NAME" b "$major" "$minor"
	if [ "$DISK_BACKEND" = nbd ]; then
		range=$(cat "/sys/class/block/$name/range")
		for ((part = 1; part < range; part++)); do
			mknod -m 0660 "$dev_root/${ALIAS_NAME}p$part" b "$major" "$((minor + part))"
		done
		while IFS='|' read -r number uuid partuuid label; do
			link_alias "$dev_root/disk/by-uuid" "$uuid" "${ALIAS_NAME}p$number"
			link_alias "$dev_root/disk/by-partuuid" "$partuuid" "${ALIAS_NAME}p$number"
			link_alias "$dev_root/disk/by-label" "$label" "${ALIAS_NAME}p$number"
		done <"$PART_MANIFEST"
	fi
	mount_tracked --bind "$dev_root" /dev
	log "private /dev: /dev/$ALIAS_NAME aliases $TARGET_DEV ($major:$minor) with $(find "$dev_root" -maxdepth 1 -type b | wc -l) block nodes and nothing else"
}

run_script_under_test() {
	(
		set +e +u +o pipefail
		export PATH="$FAKE_BIN:$PATH" FAKE_EFI_STATE
		source "$SOURCEABLE_SCRIPT"
		get_serial_port() {
			serial_port=ttyS0
			serial_port_num=0
			log_output=/dev/null
			echo "Using serial port: [$serial_port] ($serial_port_num) set by the loopback driver"
		}
		udevadm() {
			if [ "${1:-}" = trigger ]; then
				echo "driver: udevadm trigger suppressed so the runner's own devices are not re-triggered"
				return 0
			fi
			command udevadm "$@"
		}
		eval "$(declare -f modify_grub_cfg | sed '1s/^modify_grub_cfg/script_modify_grub_cfg/')"
		modify_grub_cfg() {
			local grub_status
			script_modify_grub_cfg
			grub_status=$?
			echo "$grub_status" >"$GRUB_RC_FILE"
			echo "driver: modify_grub_cfg returned $grub_status"
			return "$grub_status"
		}
		eval "$(declare -f add_cloud_init | sed '1s/^add_cloud_init/script_add_cloud_init/')"
		add_cloud_init() {
			echo invoked >"$SEED_CALL_FILE"
			script_add_cloud_init
		}
		cd "$DOWNLOAD_DIR"
		main
		echo "$?" >"$MAIN_RC_FILE"
	) >"$SCRIPT_LOG" 2>&1 || true
}

build_private_dev
log "running $VARIANT disk_imaging.sh main() with image_disk=/dev/$ALIAS_NAME"
run_script_under_test
ALIAS_ROOT_DEVICE=$(blkid -c /dev/null -t "UUID=$ROOT_UUID" -o device 2>/dev/null | head -n 1 || true)
release_mount /dev
release_mount "$WORK_DIR/scratch/dev"
unmount_below /mnt
for attempt in 1 2 3 4 5; do
	[ "$(top_mount_source /mnt)" = imager-ci-mnt ] && break
	add_note "script left a filesystem mounted on /mnt"
	umount /mnt 2>/dev/null || umount -l /mnt || true
done

MAIN_RC=$(cat "$MAIN_RC_FILE" 2>/dev/null || true)
GRUB_RC=$(cat "$GRUB_RC_FILE" 2>/dev/null || true)
[ -n "$MAIN_RC" ] || add_note "main did not return (the script called exit); see script-output.log"
[ -n "$GRUB_RC" ] || add_note "modify_grub_cfg was never reached"
[ -f "$SEED_CALL_FILE" ] || add_note "add_cloud_init was not called"
BOOT_DECISION=$(classify_boot_decision "$SCRIPT_LOG" "$IMAGE_GRUB_IN_ROOT" "$GENERATION")
log "main rc=${MAIN_RC:-exit} modify_grub_cfg rc=${GRUB_RC:-none} boot decision=$BOOT_DECISION alias root=${ALIAS_ROOT_DEVICE:-none}"

INFRA_LIMITATION=
HARNESS_ERRORS=()
if [ "$DISK_BACKEND" = loop ] && grep -q 'is not a whole-disk block device' "$SCRIPT_LOG"; then
	INFRA_LIMITATION="loop backend: lsblk reports TYPE loop and this variant only images whole disks"
elif grep -q "^Imaging .* to /dev/$ALIAS_NAME" "$SCRIPT_LOG" && [[ ! "$ALIAS_ROOT_DEVICE" =~ ^/dev/${ALIAS_NAME}p[0-9]+$ ]]; then
	if [ "$DISK_BACKEND" = loop ]; then
		INFRA_LIMITATION="loop backend: partitions get dynamic minors, so no /dev/${ALIAS_NAME}pN alias existed for the script"
	else
		HARNESS_ERRORS+=("blkid named the imaged root '${ALIAS_ROOT_DEVICE:-nothing}' instead of a /dev/${ALIAS_NAME}pN alias")
	fi
fi

FAKE_EFI_MODE=report FAKE_EFI_STATE="$FAKE_EFI_STATE" python3 -I "$IMAGER_TEST_DIR/fake_efibootmgr.py" >"$LOG_DIR/efi-report.json" || die "fake efibootmgr report failed"

if [ -n "$INFRA_LIMITATION" ]; then
	SEED_WRITTEN=false
	add_note "imaged root not inspected: $INFRA_LIMITATION"
else
	inspect_imaged_root "$TARGET_DEV" "$ROOT_UUID"
fi
detach_block_device "$TARGET_DEV"

BOOTED=skipped
if [ "$BOOT_TEST" = 1 ] && [ "$SEED_WRITTEN" = true ]; then
	boot_imaged_disk "$TARGET_RAW" "$LOG_DIR/boot-serial.log" "$BOOT_TIMEOUT" "$WORK_DIR/scratch/ovmf-vars.fd" user
elif [ "$BOOT_TEST" = 1 ]; then
	add_note "boot test skipped: no cloud-init seed, so nothing would emit the marker"
fi

result_str image "$IMAGE_NAME"
result_str variant "$VARIANT"
result_str level level1
result_str boot_decision "$BOOT_DECISION"
result_json grub_rc "${GRUB_RC:-null}"
result_json main_rc "${MAIN_RC:-null}"
result_json seed_written "$SEED_WRITTEN"
if [ "$BOOTED" = skipped ]; then result_str booted skipped; else result_json booted "$BOOTED"; fi
result_json duration_s "$((SECONDS - START_SECONDS))"
result_str disk_backend "$DISK_BACKEND"
result_str root_uuid "$ROOT_UUID"
result_json image_grub_in_root "$IMAGE_GRUB_IN_ROOT"
result_json alias "$(jq -cn --arg disk "/dev/$ALIAS_NAME" --arg backing "$TARGET_DEV" --arg root "$ALIAS_ROOT_DEVICE" '{image_disk: $disk, backing_device: $backing, root_device: $root}')"
result_json efi "$(jq -c 'del(.calls)' "$LOG_DIR/efi-report.json")"
result_str infra_limitation "$INFRA_LIMITATION"
result_json harness_errors "$(printf '%s\n' "${HARNESS_ERRORS[@]}" | jq -Rsc 'split("\n") | map(select(length > 0))')"
write_result "$RESULT_PATH"
