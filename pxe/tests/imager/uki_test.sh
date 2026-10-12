#!/bin/bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

UKI_PATH=
VARIANT=uki
IMAGE_NAME=
IMAGE_FILE=
RESULT_PATH=
WORK_DIR_REQUEST=
BOOT_TEST=1
BOOT_TIMEOUT=
IMAGER_TIMEOUT=
IMAGER_MEMORY_MB=6144
DISK_BACKEND_REQUEST=auto
TARGET_SIZE=16G
CMDLINE_METHOD=auto
NIC_MAC=52:54:00:c1:0d:01

usage() {
	cat >&2 <<EOF
usage: sudo unshare -m --propagation private bash $0 --uki qcow-imager.efi --image NAME --image-file FILE --work-dir DIR --result result.json [options]
  --variant NAME         label for the result (default uki, e.g. uki:v2.2.0-vcluster.3.g20eef036)
  --log-dir DIR          where logs land (default: directory of --result)
  --boot-test 0|1        boot the imaged disk afterwards with the imager's NVRAM and no NIC (default 1)
  --boot-timeout SECS    QEMU boot timeout (default 600 with KVM, 1500 without)
  --imager-timeout SECS  UKI imaging run timeout (default 1500 with KVM, 3000 without)
  --imager-memory MB     RAM for the imager VM, must hold the downloaded image (default $IMAGER_MEMORY_MB)
  --cmdline-method MODE  auto|kernel|smbios how the NICo cmdline reaches systemd-stub (default auto)
  --disk-backend MODE    auto|nbd|loop block device used to probe and inspect disks (default auto)
  --target-size SIZE     sparse target disk size (default $TARGET_SIZE)
EOF
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--uki) UKI_PATH=$2; shift 2 ;;
		--variant) VARIANT=$2; shift 2 ;;
		--image) IMAGE_NAME=$2; shift 2 ;;
		--image-file) IMAGE_FILE=$2; shift 2 ;;
		--work-dir) WORK_DIR_REQUEST=$2; shift 2 ;;
		--result) RESULT_PATH=$2; shift 2 ;;
		--log-dir) LOG_DIR=$2; shift 2 ;;
		--boot-test) BOOT_TEST=$2; shift 2 ;;
		--boot-timeout) BOOT_TIMEOUT=$2; shift 2 ;;
		--imager-timeout) IMAGER_TIMEOUT=$2; shift 2 ;;
		--imager-memory) IMAGER_MEMORY_MB=$2; shift 2 ;;
		--cmdline-method) CMDLINE_METHOD=$2; shift 2 ;;
		--disk-backend) DISK_BACKEND_REQUEST=$2; shift 2 ;;
		--target-size) TARGET_SIZE=$2; shift 2 ;;
		*) usage ;;
	esac
done

[ -n "$UKI_PATH" ] && [ -n "$IMAGE_NAME" ] && [ -n "$IMAGE_FILE" ] && [ -n "$WORK_DIR_REQUEST" ] && [ -n "$RESULT_PATH" ] || usage
[ -n "$LOG_DIR" ] || LOG_DIR=$(dirname "$RESULT_PATH")
mkdir -p "$LOG_DIR"
[ -f "$UKI_PATH" ] || die "UKI not found: $UKI_PATH"
[ -f "$IMAGE_FILE" ] || die "image not found: $IMAGE_FILE"

require_root
install_cleanup_trap
enter_private_namespace
require_tools qemu-img qemu-nbd qemu-system-x86_64 blkid losetup findmnt mountpoint jq python3 curl timeout udevadm partprobe mformat mmd mcopy
START_SECONDS=$SECONDS
prepare_work_dir "$WORK_DIR_REQUEST"
IMAGE_FILE=$(readlink -f -- "$IMAGE_FILE")
UKI_PATH=$(readlink -f -- "$UKI_PATH")
TARGET_RAW="$WORK_DIR/scratch/target.raw"
OVMF_VARS="$WORK_DIR/scratch/ovmf-vars.fd"
IMAGER_SERIAL="$LOG_DIR/imager-serial.log"
BOOT_TIMEOUT=${BOOT_TIMEOUT:-$(default_timeout 600 1500)}
IMAGER_TIMEOUT=${IMAGER_TIMEOUT:-$(default_timeout 1500 3000)}
[ -w /dev/kvm ] || add_note "no /dev/kvm, QEMU runs with TCG"

DISK_BACKEND=$(choose_disk_backend "$DISK_BACKEND_REQUEST")
IMAGE_FORMAT=$(qemu-img info --output=json "$IMAGE_FILE" | jq -r '.format')
attach_block_device "$IMAGE_FILE" "$IMAGE_FORMAT" ro "$DISK_BACKEND"
PROBE_DEV=$ATTACHED_DEVICE
probe_source_image "$PROBE_DEV"
detach_block_device "$PROBE_DEV"

serve_image_and_seed "$IMAGE_FILE"
NICO_CMDLINE="loglevel=7 console=tty0 pci=realloc=off console=ttyS0,115200 mac=$NIC_MAC image_url=http://10.0.2.2:$HTTP_PORT/$(basename "$IMAGE_FILE") image_sha=$IMAGE_SHA rootfs_uuid=$ROOT_UUID image_disk=/dev/nvme0n1 ds=nocloud-net;s=http://10.0.2.2:$HTTP_PORT/seed"
printf '%s\n' "$NICO_CMDLINE" >"$LOG_DIR/kernel-cmdline"
OVMF_CODE=$(ovmf_code)
VARS_TEMPLATE=$(ovmf_vars_template)

build_uki_esp_image() {
	local esp_image=$1 esp_mb
	esp_mb=$(( $(stat -c %s "$UKI_PATH") / 1024 / 1024 + 96 ))
	rm -f "$esp_image"
	truncate -s "${esp_mb}M" "$esp_image"
	mformat -i "$esp_image" -F ::
	mmd -i "$esp_image" ::/EFI ::/EFI/BOOT
	mcopy -i "$esp_image" "$UKI_PATH" ::/EFI/BOOT/BOOTX64.EFI
}

run_imager_vm() {
	local method=$1 accel_args method_args esp_image
	accel_args=$(qemu_accel_args)
	cp "$VARS_TEMPLATE" "$OVMF_VARS"
	rm -f "$TARGET_RAW"
	truncate -s "$TARGET_SIZE" "$TARGET_RAW"
	case "$method" in
		kernel)
			method_args=(-kernel "$UKI_PATH" -append "$NICO_CMDLINE")
			;;
		smbios)
			esp_image="$WORK_DIR/scratch/uki-esp.img"
			build_uki_esp_image "$esp_image"
			method_args=(-drive "file=$esp_image,format=raw,if=virtio,readonly=on" -smbios "type=11,value=io.systemd.stub.kernel-cmdline-extra=${NICO_CMDLINE//,/,,}")
			;;
		*)
			die "unknown cmdline method $method"
			;;
	esac
	log "imager VM via $method (timeout ${IMAGER_TIMEOUT}s, ${IMAGER_MEMORY_MB}M, $accel_args)"
	: >"$IMAGER_SERIAL"
	IMAGER_QEMU_RC=0
	timeout "$IMAGER_TIMEOUT" qemu-system-x86_64 \
		$accel_args -smp 2 -m "$IMAGER_MEMORY_MB" \
		-display none -monitor none -no-reboot \
		-drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
		-drive if=pflash,format=raw,file="$OVMF_VARS" \
		-drive file="$TARGET_RAW",format=raw,if=none,id=target,discard=unmap,detect-zeroes=unmap \
		-device nvme,drive=target,serial=imagerci \
		-netdev user,id=n0 -device virtio-net-pci,netdev=n0,mac="$NIC_MAC" \
		-serial file:"$IMAGER_SERIAL" \
		"${method_args[@]}" \
		>"$LOG_DIR/qemu-imager-$method.stdout" 2>&1 &
	CLEANUP_PIDS+=("$!")
	wait "$!" || IMAGER_QEMU_RC=$?
}

IMAGER_METHOD=$CMDLINE_METHOD
[ "$IMAGER_METHOD" != auto ] || IMAGER_METHOD=kernel
run_imager_vm "$IMAGER_METHOD"
if [ "$CMDLINE_METHOD" = auto ] && ! grep -aq 'image_url=' "$IMAGER_SERIAL"; then
	add_note "-kernel/-append did not reach systemd-stub; retried via smbios kernel-cmdline-extra"
	cp "$IMAGER_SERIAL" "$LOG_DIR/imager-serial.kernel-method.log"
	IMAGER_METHOD=smbios
	run_imager_vm "$IMAGER_METHOD"
fi
add_note "cmdline delivered via $IMAGER_METHOD; main_rc is the imager VM's qemu exit code"
[ "$IMAGER_QEMU_RC" -ne 124 ] || add_note "imager VM timed out after ${IMAGER_TIMEOUT}s"
grep -aq 'Rebooting' "$IMAGER_SERIAL" || add_note "imager never logged Rebooting"

BOOT_DECISION=$(classify_boot_decision "$IMAGER_SERIAL" "$IMAGE_GRUB_IN_ROOT" unknown)
GRUB_RC=null
if grep -aq 'Updating grub configuration' "$IMAGER_SERIAL"; then
	GRUB_SECTION=$(tr -d '\r' <"$IMAGER_SERIAL" | sed -En '/Updating grub configuration/,/fetching from cloud-init url|^Rebooting/p')
	if grep -qE '^done[[:space:]]*$' <<<"$GRUB_SECTION"; then
		GRUB_RC=0
	else
		GRUB_RC=1
	fi
	add_note "grub_rc inferred from whether grub-mkconfig printed its done line"
elif grep -aq 'grub.cfg not found' "$IMAGER_SERIAL"; then
	GRUB_RC=0
	add_note "grub_rc inferred from the grub.cfg-not-found early return"
fi
BOOT_ORDER_LINES=$(tr -d '\r' <"$IMAGER_SERIAL" | { grep -aoE '(Setting boot order to: [0-9A-Fa-f,]+|Boot order already network-first then Boot[^,]*, no change needed)' || true; } | awk '!seen[$0]++' | jq -Rsc 'split("\n") | map(select(length > 0))')
log "boot decision: $BOOT_DECISION grub_rc(inferred)=$GRUB_RC boot order lines: $BOOT_ORDER_LINES"

attach_block_device "$TARGET_RAW" raw ro "$DISK_BACKEND"
TARGET_DEV=$ATTACHED_DEVICE
inspect_imaged_root "$TARGET_DEV" "$ROOT_UUID"
if [ "$SEED_WRITTEN" = true ] && [ "$GRUB_RC" != 0 ]; then
	GRUB_RC=0
	add_note "grub_rc set to 0: the seed was written and these imagers stop before the seed when grub fails"
fi
detach_block_device "$TARGET_DEV"

BOOTED=skipped
if [ "$BOOT_TEST" = 1 ] && [ "$SEED_WRITTEN" = true ]; then
	boot_imaged_disk "$TARGET_RAW" "$LOG_DIR/boot-serial.log" "$BOOT_TIMEOUT" "$OVMF_VARS" none
elif [ "$BOOT_TEST" = 1 ]; then
	add_note "boot test skipped: no cloud-init seed, so nothing would emit the marker"
fi

result_str image "$IMAGE_NAME"
result_str variant "$VARIANT"
result_str level level2
result_str boot_decision "$BOOT_DECISION"
result_json grub_rc "$GRUB_RC"
result_json main_rc "$IMAGER_QEMU_RC"
result_json seed_written "$SEED_WRITTEN"
if [ "$BOOTED" = skipped ]; then result_str booted skipped; else result_json booted "$BOOTED"; fi
result_json duration_s "$((SECONDS - START_SECONDS))"
result_str disk_backend "qemu-nvme/$DISK_BACKEND"
result_str root_uuid "$ROOT_UUID"
result_json image_grub_in_root "$IMAGE_GRUB_IN_ROOT"
result_json boot_order_lines "$BOOT_ORDER_LINES"
result_str infra_limitation ""
result_json harness_errors "[]"
write_result "$RESULT_PATH"
