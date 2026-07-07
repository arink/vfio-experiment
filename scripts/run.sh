#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/qemu/build/qemu-system-x86_64" \
  -d guest_errors -D $SCRIPT_DIR/qemu-debug.log \
  -L "$SCRIPT_DIR/qemu/pc-bios" \
  -kernel "$SCRIPT_DIR/linux/arch/x86/boot/bzImage" \
  -drive file="$SCRIPT_DIR/buildroot/output/images/rootfs.ext2",format=raw,if=virtio \
  -append "root=/dev/vda rw console=ttyS0 nokaslr intel_iommu=on iommu=pt vfio_pci.enable_sriov=1 vfio-pci.ids=0x1234:0xcafe" \
  -nographic \
  -m 2G \
  -smp 1 \
  -machine q35,accel=kvm,kernel-irqchip=split \
  -cpu host \
  -device intel-iommu,intremap=on,device-iotlb=on \
  -device ioh3420,id=pcie.1,chassis=1,slot=1,bus=pcie.0,addr=1c.0 \
  -device experiment,id=experiment,bus=pcie.1,addr=0 \
  -virtfs local,path=$SCRIPT_DIR/..,mount_tag=host0,security_model=mapped \
  -nic user,model=virtio-net-pci
