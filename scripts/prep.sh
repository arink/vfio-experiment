#!/bin/bash
set -euo pipefail

#
# QEMU
#
if [ ! -d "qemu" ]; then
    git clone --branch experiment https://github.com/arink/qemu.git
    pushd qemu
    ./configure --enable-rust --enable-slirp
    make -j$(nproc)
    popd qemu
fi

#
# Linux Kernel
#
if [ ! -d "linux" ]; then
    git clone --depth=1 --branch v7.1 https://github.com/torvalds/linux.git
    pushd linux
    make defconfig
    scripts/config -e CONFIG_VFIO
    scripts/config -e CONFIG_VFIO_PCI
    scripts/config -e CONFIG_VFIO_IOMMU_TYPE_1
    scripts/config -e CONFIG_INTEL_IOMMU_DEFAULT_ON
    scripts/config -e CONFIG_VFIO_MDEV
    scripts/config -e CONFIG_VFIO_NOIOMMU
    scripts/config -e CONFIG_9P_FS
    scripts/config -e CONFIG_9P_FS_POSIX_ACL
    scripts/config -e CONFIG_VIRTIO_PCI
    scripts/config -e CONFIG_NET_9P_VIRTIO
    make olddefconfig
    make -j$(nproc) bzImage
    popd
fi


#
# Buildroot
#
if [ ! -d "buildroot" ]; then
    # Tested against c5728ff5af9898ab1db967d7e13af2b17825ed25 from 2026/07/04 
    git clone --depth=1 https://github.com/buildroot/buildroot.git
    pushd buildroot
    make qemu_x86_64_defconfig
    ./utils/config --enable BR2_TOOLCHAIN_BUILDROOT_CXX
    ./utils/config --enable BR2_INSTALL_LIBSTDCPP
    ./utils/config --enable BR2_PACKAGE_PCIUTILS
    ./utils/config --enable BR2_PACKAGE_STRACE
    ./utils/config --enable BR2_PACKAGE_RIPGREP
    ./utils/config --enable BR2_PACKAGE_MAKE
    ./utils/config --enable BR2_PACKAGE_HOST_CMAKE
    ./utils/config --enable BR2_PACKAGE_HOST_ENVIRONMENT_SETUP
    ./utils/config --enable BR2_PACKAGE_HOST_RUSTC
    ./utils/config --enable BR2_PACKAGE_HOST_RUST_BIN
    ./utils/config --enable BR2_OPTIMIZE_2
    ./utils/config --set-str BR2_TARGET_ROOTFS_EXT2_SIZE "256M"
    make olddefconfig
    
    # Automount
    cat updates/buildroot_post_build.txt >> buildroot/board/qemu/x86_64/post-build.sh

    # Build target and sdk
    make -j$(nproc)
    make -j$(nproc) sdk
    popd
fi

echo Prep complete!
