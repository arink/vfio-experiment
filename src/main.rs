//! Minimal VFIO userspace driver.
//!
//! Finds a PCI device by vendor/device ID, binds it through VFIO,
//! maps BAR0, and reads two 32-bit registers at offset 0x0000 and 0x0004.
//!
//! Prerequisites (must be done before running, typically as root):
//!   1. IOMMU enabled in kernel: `intel_iommu=on` or `amd_iommu=on` (+ `iommu=pt` optionally).
//!   2. Device bound to the `vfio-pci` driver instead of its normal driver, e.g.:
//!        DEV=0000:01:00.0
//!        echo $DEV > /sys/bus/pci/devices/$DEV/driver/unbind   # if bound to something else
//!        echo vfio-pci > /sys/bus/pci/devices/$DEV/driver_override
//!        echo $DEV > /sys/bus/pci/drivers_probe
//!   3. Permission to access /dev/vfio/vfio and /dev/vfio/<group_id>
//!      (root, or chown/chmod them, or add your user via appropriate group).
//!
//! Run as root:
//!   cargo build --release
//!   sudo ./target/release/vfio-driver

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;

const TARGET_VENDOR: u16 = 0x1234;
const TARGET_DEVICE: u16 = 0xCAFE;

// ---- VFIO ioctl definitions (from linux/vfio.h) ----
// All VFIO ioctls use the plain _IO() encoding: (type << 8) | nr
const VFIO_TYPE: u32 = b';' as u32;
const VFIO_BASE: u32 = 100;

const fn vfio_io(nr: u32) -> u64 {
    ((VFIO_TYPE << 8) | (VFIO_BASE + nr)) as u64
}

const VFIO_GET_API_VERSION: u64 = vfio_io(0);
const VFIO_CHECK_EXTENSION: u64 = vfio_io(1);
const VFIO_SET_IOMMU: u64 = vfio_io(2);
const VFIO_GROUP_GET_STATUS: u64 = vfio_io(3);
const VFIO_GROUP_SET_CONTAINER: u64 = vfio_io(4);
const VFIO_GROUP_GET_DEVICE_FD: u64 = vfio_io(6);
const VFIO_DEVICE_GET_INFO: u64 = vfio_io(7);
const VFIO_DEVICE_GET_REGION_INFO: u64 = vfio_io(8);

const VFIO_API_VERSION: i32 = 0;
const VFIO_TYPE1_IOMMU: u64 = 1;

const VFIO_GROUP_FLAGS_VIABLE: u32 = 1 << 0;

const VFIO_REGION_INFO_FLAG_MMAP: u32 = 1 << 2;

const VFIO_PCI_BAR0_REGION_INDEX: u32 = 0;

#[repr(C)]
#[derive(Debug, Default)]
struct VfioGroupStatus {
    argsz: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Debug, Default)]
struct VfioDeviceInfo {
    argsz: u32,
    flags: u32,
    num_regions: u32,
    num_irqs: u32,
}

#[repr(C)]
#[derive(Debug, Default)]
struct VfioRegionInfo {
    argsz: u32,
    flags: u32,
    index: u32,
    cap_offset: u32,
    size: u64,
    offset: u64,
}

fn ioctl_raw(fd: RawFd, request: u64, arg: usize) -> io::Result<i32> {
    let ret = unsafe { libc::ioctl(fd, request as libc::c_ulong, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// Search /sys/bus/pci/devices for a device matching vendor:device ID.
/// Returns the PCI BDF string (e.g. "0000:01:00.0").
fn find_pci_device(vendor: u16, device: u16) -> io::Result<String> {
    let base = Path::new("/sys/bus/pci/devices");
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let bdf = entry.file_name().into_string().unwrap();
        let dev_path = entry.path();

        let read_id = |name: &str| -> Option<u16> {
            let s = fs::read_to_string(dev_path.join(name)).ok()?;
            u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
        };

        if let (Some(v), Some(d)) = (read_id("vendor"), read_id("device")) {
            if v == vendor && d == device {
                return Ok(bdf);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "no PCI device found with vendor=0x{:04x} device=0x{:04x}",
            vendor, device
        ),
    ))
}

/// Resolve the IOMMU group number for a given PCI BDF.
fn iommu_group_for(bdf: &str) -> io::Result<String> {
    let link_path = format!("/sys/bus/pci/devices/{}/iommu_group", bdf);
    let target = fs::read_link(&link_path)?;
    let group = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad iommu_group symlink"))?
        .to_string_lossy()
        .to_string();
    Ok(group)
}

fn main() -> io::Result<()> {
    // 1. Find the target device and its IOMMU group.
    let bdf = find_pci_device(TARGET_VENDOR, TARGET_DEVICE)?;
    let group_id = iommu_group_for(&bdf)?;
    println!("Found device {} (vendor=0x{:04x} device=0x{:04x}) in IOMMU group {}",
        bdf, TARGET_VENDOR, TARGET_DEVICE, group_id);

    // 2. Open the VFIO container.
    let container_path = CString::new("/dev/vfio/vfio").unwrap();
    let container_fd = unsafe { libc::open(container_path.as_ptr(), libc::O_RDWR) };
    if container_fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let api_version = ioctl_raw(container_fd, VFIO_GET_API_VERSION, 0)?;
    if api_version != VFIO_API_VERSION {
        panic!("unexpected VFIO API version: {}", api_version);
    }

    let has_type1 = ioctl_raw(container_fd, VFIO_CHECK_EXTENSION, VFIO_TYPE1_IOMMU as usize)?;
    if has_type1 == 0 {
        panic!("VFIO_TYPE1_IOMMU not supported on this system");
    }

    // 3. Open the group.
    let group_path = CString::new(format!("/dev/vfio/{}", group_id)).unwrap();
    let group_fd = unsafe { libc::open(group_path.as_ptr(), libc::O_RDWR) };
    if group_fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut status = VfioGroupStatus {
        argsz: std::mem::size_of::<VfioGroupStatus>() as u32,
        flags: 0,
    };
    ioctl_raw(group_fd, VFIO_GROUP_GET_STATUS, &mut status as *mut _ as usize)?;
    if status.flags & VFIO_GROUP_FLAGS_VIABLE == 0 {
        panic!(
            "IOMMU group {} is not viable (some devices in it are not bound to vfio-pci)",
            group_id
        );
    }

    // 4. Attach the group to the container, then set the IOMMU type.
    ioctl_raw(group_fd, VFIO_GROUP_SET_CONTAINER, &container_fd as *const _ as usize)?;
    ioctl_raw(container_fd, VFIO_SET_IOMMU, VFIO_TYPE1_IOMMU as usize)?;

    // 5. Get a device fd for our specific BDF from the group.
    let bdf_c = CString::new(bdf.clone()).unwrap();
    let device_fd = ioctl_raw(group_fd, VFIO_GROUP_GET_DEVICE_FD, bdf_c.as_ptr() as usize)?;

    let mut dev_info = VfioDeviceInfo {
        argsz: std::mem::size_of::<VfioDeviceInfo>() as u32,
        ..Default::default()
    };
    ioctl_raw(device_fd, VFIO_DEVICE_GET_INFO, &mut dev_info as *mut _ as usize)?;
    println!(
        "Device has {} regions, {} irqs",
        dev_info.num_regions, dev_info.num_irqs
    );

    // 6. Query BAR0 region info.
    let mut region_info = VfioRegionInfo {
        argsz: std::mem::size_of::<VfioRegionInfo>() as u32,
        index: VFIO_PCI_BAR0_REGION_INDEX,
        ..Default::default()
    };
    ioctl_raw(
        device_fd,
        VFIO_DEVICE_GET_REGION_INFO,
        &mut region_info as *mut _ as usize,
    )?;

    if region_info.flags & VFIO_REGION_INFO_FLAG_MMAP == 0 {
        panic!("BAR0 does not support mmap");
    }
    if region_info.size < 8 {
        panic!("BAR0 is smaller than 8 bytes, cannot read offset 0x0004");
    }

    // 7. mmap BAR0.
    let map_len = region_info.size as usize;
    let map_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            device_fd,
            region_info.offset as libc::off_t,
        )
    };
    if map_ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }

    // 8. Read the two registers.
    // Safety: map_ptr is valid for map_len bytes, and map_len >= 8 (checked above).
    let reg0 = unsafe { std::ptr::read_volatile((map_ptr as *const u32).offset(0)) };
    let reg1 = unsafe { std::ptr::read_volatile((map_ptr as *const u32).offset(1)) };

    println!("Register @0x0000: 0x{:08x}", reg0);
    println!("Register @0x0004: 0x{:08x}", reg1);

    // Cleanup.
    unsafe {
        libc::munmap(map_ptr, map_len);
        libc::close(device_fd);
        libc::close(group_fd);
        libc::close(container_fd);
    }

    Ok(())
}