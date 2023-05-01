//! A simple SATA AHCI driver.
//! Does not support port multipliers.
//! Currently limited to one command.

use crate::arch;
use crate::bio;
use crate::fs;
use crate::kalloc;
use crate::kmem;
use crate::pci;
use crate::spinlock::SpinMutex as Mutex;
use crate::xapic;
use bitflags::bitflags;
use core::convert::TryFrom;
use core::convert::TryInto;
use core::mem;
use core::time::Duration;
use static_assertions::const_assert_eq;

// Private helper functions.
mod volatile {
    use core::ops::{BitAnd, BitOr, Not};
    use core::ptr;

    pub fn write<T>(r: &mut T, v: T) {
        unsafe {
            ptr::write_volatile(r, v);
        }
    }

    pub fn read<T: BitOr<Output = T> + BitAnd<Output = T> + Not<Output = T>>(r: &T) -> T {
        unsafe { ptr::read_volatile(r) }
    }

    pub fn set<T: BitOr<Output = T> + BitAnd<Output = T> + Not<Output = T>>(r: &mut T, v: T) {
        let tmp = read(r);
        write(r, tmp | v);
    }

    pub fn clear<T: BitAnd<Output = T> + BitOr<Output = T> + Not<Output = T>>(r: &mut T, v: T) {
        let tmp = read(r);
        write(r, tmp & !v);
    }
}

mod fis {
    /// Frame Information Structure types.
    #[repr(u8)]
    pub(super) enum Type {
        HostToDevReg = 0x27,
        _DevToHostReg = 0x34,
        _DMAActivate = 0x39,
        _FirstPartyDMASetup = 0x41,
        _Data = 0x46,
        _BISTActivate = 0x58,
        _PIOSetup = 0x5f,
        _SetDevBits = 0xa1,
    }

    /// A host to device register.
    #[derive(Clone, Copy, Default, Debug)]
    #[repr(C)]
    pub(super) struct RegH2D {
        // u32 0
        fis_type: u8,
        crrr_port: u8,
        command: u8,
        features0: u8,
        // u32 1
        lba0: u8,
        lba1: u8,
        lba2: u8,
        device: u8,
        // u32 2
        lba3: u8,
        lba4: u8,
        lba5: u8,
        features1: u8,
        // u32 3
        count0: u8,
        count1: u8,
        icc: u8,
        control: u8,
        // u32 4
        aux0: u8,
        aux1: u8,
        aux2: u8,
        aux3: u8,
    }

    impl RegH2D {
        pub(super) fn new() -> Self {
            Self {
                fis_type: Type::HostToDevReg as u8,
                ..Self::default()
            }
        }

        pub(super) fn with_lba(self, lba: u64) -> Self {
            Self {
                lba0: lba as u8,
                lba1: (lba >> 8) as u8,
                lba2: (lba >> 16) as u8,
                lba3: (lba >> 24) as u8,
                lba4: (lba >> 32) as u8,
                lba5: (lba >> 40) as u8,
                ..self
            }
        }

        pub(super) fn with_device_lba(self) -> Self {
            const DEV_MB1: u8 = 1 << 6;
            Self {
                device: DEV_MB1,
                ..self
            }
        }

        pub(super) fn with_cflag(self) -> Self {
            Self {
                crrr_port: self.crrr_port | 0b1000_0000,
                ..self
            }
        }

        pub(super) fn with_command(self, cmd: super::ATACommand) -> Self {
            Self {
                command: cmd as u8,
                ..self
            }
        }

        pub(super) fn with_count(self, count: u16) -> Self {
            Self {
                count0: count as u8,
                count1: (count >> 8) as u8,
                ..self
            }
        }
    }

    /// A device to host register.
    #[derive(Default)]
    #[repr(C)]
    pub(super) struct RegD2H {
        // u32 0
        fis_type: u8,
        rirr_port: u8,
        status: u8,
        error: u8,
        // u32 1
        lba0: u8,
        lba1: u8,
        lba2: u8,
        device: u8,
        // u32 2
        lba3: u8,
        lba4: u8,
        lba5: u8,
        _res0: u8,
        // u32 3
        count0: u8,
        count1: u8,
        _res1: u16,
        // u32 4
        _res2: u32,
    }
}

const SECTOR_SIZE: usize = 512;

bitflags! {
    pub struct GlobalHBACtl: u32 {
        const HBA_RESET = 1;
        const INTR_ENABLE = 1 << 1;
        const MSI_REVERT_SINGLE_MSG = 1 << 2;
        const AHCI_ENABLE = 1 << 31;
    }
}

#[derive(Debug)]
#[repr(C)]
struct GenericHostCtl {
    cap: u32,
    ghc: u32,
    intr_status: u32,
    ports_impl: u32,
    version: u32,
    ccc_ctl: u32,
    ccc_ports: u32,
    encl_mgmt_loc: u32,
    encl_mgmt_ctl: u32,
    cap2: u32,
    bohc: u32,
    res0: [u32; 13],
    res_nvmhci: [u32; 16],
    vendor: [u32; 24],
    port: [Port; 32],
}

impl GenericHostCtl {
    fn eoi(&mut self) {
        volatile::write(&mut self.intr_status, 0);
    }
}

bitflags! {
    pub struct PortIntrEnable: u32 {
        const DEV_HOST_REG_FIS = 1;
    }
}

bitflags! {
    pub struct TaskFileData: u32 {
        const DRQ = 1 << 3;
        const BUSY = 1 << 7;
    }
}

#[derive(Default, Debug)]
#[repr(C, align(128))]
struct Port {
    cmd_base_lo: u32,
    cmd_base_hi: u32,
    fis_base_lo: u32,
    fis_base_hi: u32,
    intr_status: u32,
    intr_enable: u32,
    cmd_status: u32,
    _res0: u32,
    task_file_data: u32,
    signature: u32,
    sata_status: u32,
    sata_control: u32,
    sata_error: u32,
    sata_active: u32,
    cmd_issue: u32,
    sata_notification: u32,
    fis_based_switching: u32,
    dev_sleep: u32,
    _res1: u32,
    vendor: u32,
}

impl Port {
    const PORT_CMD_CR: u32 = 1 << 15;
    const PORT_CMD_FR: u32 = 1 << 14;
    const PORT_CMD_FRE: u32 = 1 << 4;
    const PORT_CMD_ST: u32 = 1;
    const PORT_BUSY: u32 =
        Self::PORT_CMD_CR | Self::PORT_CMD_FR | Self::PORT_CMD_FRE | Self::PORT_CMD_ST;

    fn is_present(&mut self) -> bool {
        let ssts = volatile::read(&self.sata_status);
        let device_detect = ssts & 0x0F;
        let iface_power_mgmt = (ssts >> 8) & 0x0F;
        device_detect == 0x03 && iface_power_mgmt == 0x01
    }

    fn is_storage(&mut self) -> bool {
        let sig = volatile::read(&self.signature);
        sig == 0x0000_0101
    }

    fn init(&mut self, drive: &mut Drive) {
        self.stop();
        let cmd_list_pa = kmem::ref_to_phys(&drive.cmd_header);
        volatile::write(&mut self.cmd_base_hi, (cmd_list_pa >> 32) as u32);
        volatile::write(&mut self.cmd_base_lo, cmd_list_pa as u32);
        let rfis_pa = kmem::ref_to_phys(&drive.rcvd_fis);
        volatile::write(&mut self.fis_base_hi, (rfis_pa >> 32) as u32);
        volatile::write(&mut self.fis_base_lo, rfis_pa as u32);
        volatile::write(
            &mut self.intr_enable,
            PortIntrEnable::DEV_HOST_REG_FIS.bits(),
        );
        self.clear_sata_error();
        self.start();
    }

    fn is_idle(&self) -> bool {
        let cmd_status = volatile::read(&self.cmd_status);
        cmd_status & Self::PORT_BUSY == 0
    }

    fn start(&mut self) {
        volatile::set(&mut self.cmd_status, Self::PORT_CMD_FRE);
        volatile::set(&mut self.cmd_status, Self::PORT_CMD_ST);
    }

    fn stop(&mut self) {
        if self.is_idle() {
            return;
        }
        volatile::clear(&mut self.cmd_status, Self::PORT_CMD_ST | Self::PORT_CMD_FRE);
        for _ in 0..500 {
            if self.is_idle() {
                return;
            }
            arch::sleep(Duration::from_micros(1));
        }
    }

    fn clear_sata_error(&mut self) {
        volatile::write(&mut self.sata_error, 0x30F0FF70);
    }

    fn issue(&mut self) {
        volatile::set(&mut self.cmd_issue, 1);
    }

    fn wait(&mut self) {
        for _ in 0..1_000_000 {
            let tfd = volatile::read(&self.task_file_data);
            let tfd = TaskFileData::from_bits_truncate(tfd);
            if !tfd.contains(TaskFileData::BUSY | TaskFileData::DRQ) {
                return;
            }
            arch::cpu_relax();
        }
        panic!("AHCI port hung");
    }

    fn is_wait_done(&self) -> bool {
        let ci = volatile::read(&self.cmd_issue);
        let tfd = volatile::read(&self.task_file_data);
        let tfd = TaskFileData::from_bits_truncate(tfd);
        !tfd.contains(TaskFileData::BUSY | TaskFileData::DRQ) && ci & 1 == 0
    }

    fn wait_done(&mut self) -> bool {
        for _ in 0..2_000_000_000 {
            if self.is_wait_done() {
                return true;
            }
            arch::cpu_relax();
        }
        false
    }

    fn eoi(&mut self) {
        volatile::write(&mut self.intr_status, !0);
    }
}

#[repr(C, align(256))]
struct RecvFIS {
    dma_setup: [u32; 7],
    _pad0: u32,
    pio_setup: [u32; 5],
    _pad1: [u32; 3],
    rfis: [u32; 5],
    _pad2: u32,
    set_dev_bits: [u32; 2],
    unknown: [u8; 64],
    _res: [u32; 24],
}
const_assert_eq!(mem::size_of::<RecvFIS>(), 256);

#[repr(u8)]
enum ATACommand {
    Identify = 0xEC,
    ReadDMAExt = 0x25,
    WriteDMAExt = 0x35,
}

#[repr(C)]
struct CmdHeader {
    // u32 0
    pwa_cfl: u8,
    pmp_rcbr: u8,
    prd_tbl_len: u16,
    // u32 1
    prd_byte_count: u32,
    // u32 2
    cmd_tbl_base_lo: u32,
    // u32 3
    cmd_tbl_base_hi: u32,
    // u32s 4-7
    _res: [u32; 4],
}
const_assert_eq!(mem::size_of::<CmdHeader>(), 32);

impl CmdHeader {
    const W: u8 = 1 << 6;

    fn set_num_prds(&mut self, nprds: u16) {
        self.prd_tbl_len = nprds;
    }

    fn set_cfl(&mut self, nu32s: usize) {
        assert!(2 <= nu32s && nu32s <= 16);
        self.pwa_cfl |= nu32s as u8;
    }

    fn set_read(&mut self) {
        self.pwa_cfl &= !Self::W;
    }

    fn set_write(&mut self) {
        self.pwa_cfl |= Self::W;
    }

    fn clear(&mut self) {
        self.prd_tbl_len = 0;
    }
}

#[repr(C, align(2))]
struct PRDTEntry {
    // u32 0
    data_phys_addr_lo: u32,
    // u32 1
    data_phys_addr_hi: u32,
    // u32 2
    _res0: u32,
    // u32 3
    data_count_i: u32,
}
const_assert_eq!(mem::size_of::<PRDTEntry>(), 16);

#[repr(C, align(128))]
struct CmdTable {
    fis: [u8; 64],
    _atapi: [u8; 16],
    _pad: [u8; 48],
    prdt: [PRDTEntry; 8], // 8 512-byte sectors = 4096 block
}
const_assert_eq!(mem::size_of::<CmdTable>(), 256);

impl CmdTable {
    fn set_prd(&mut self, buf: &[u8]) {
        let pa = kmem::ptr_to_phys(buf.as_ptr());
        assert_eq!(pa & 1, 0, "misaligned prd");
        volatile::write(&mut self.prdt[0].data_phys_addr_hi, (pa >> 32) as u32);
        volatile::write(&mut self.prdt[0].data_phys_addr_lo, pa as u32);
        volatile::write(&mut self.prdt[0].data_count_i, buf.len() as u32 - 1);
    }

    fn set_command_fis(&mut self, fis: fis::RegH2D) {
        volatile::write(
            unsafe { &mut *(self.fis.as_mut_ptr() as *mut fis::RegH2D) },
            fis,
        );
    }
}

#[repr(C, align(4096))]
struct Drive {
    cmd_header: CmdHeader,
    _unused_cmd_hdrs: [CmdHeader; 31],
    rcvd_fis: RecvFIS,
    cmd_table: CmdTable,
    identity: [u8; SECTOR_SIZE],
    sectors: u64,
    port: *mut Port,
    ctlr: *mut GenericHostCtl,
    model: [u8; 40],
    serial: [u8; 20],
}
const_assert_eq!(mem::size_of::<Drive>(), 4096);

impl Drive {
    fn new(port: &mut Port, ctlr: *mut GenericHostCtl) -> &'static mut Drive {
        let page: &mut crate::arch::Page = kalloc::alloc().expect("allocated a per-port ACHI page");
        let drive = unsafe { mem::transmute::<_, &'static mut Drive>(page.as_ptr_mut()) };
        let phys_tbl = kmem::ref_to_phys(&drive.cmd_table);
        volatile::write(
            &mut drive.cmd_header.cmd_tbl_base_hi,
            (phys_tbl >> 32) as u32,
        );
        volatile::write(&mut drive.cmd_header.cmd_tbl_base_lo, phys_tbl as u32);
        port.init(drive);
        drive.port = port;
        drive.ctlr = ctlr;
        drive.identify();

        drive.serial.copy_from_slice(&drive.identity[20..40]);
        drive.serial.chunks_mut(2).for_each(|c| c.reverse());
        let serial = unsafe { core::str::from_utf8_unchecked(&drive.serial).trim() };

        drive.model.copy_from_slice(&drive.identity[54..94]);
        drive.model.chunks_mut(2).for_each(|c| c.reverse());
        let model = unsafe { core::str::from_utf8_unchecked(&drive.model).trim() };

        let sectors = u64::from_le_bytes((&drive.identity[200..208]).try_into().unwrap());
        drive.sectors = sectors;
        crate::println!("drive model '{model}', serial '{serial}', sectors {sectors}");

        drive
    }

    fn setup_read_cmd(&mut self, fis: fis::RegH2D) {
        self.cmd_table.set_command_fis(fis);
        self.cmd_header.set_num_prds(1);
        self.cmd_header.set_read();
        self.cmd_header
            .set_cfl(mem::size_of::<fis::RegH2D>() / mem::size_of::<u32>());
    }

    fn setup_write_cmd(&mut self, fis: fis::RegH2D) {
        self.cmd_table.set_command_fis(fis);
        self.cmd_header.set_num_prds(1);
        self.cmd_header.set_write();
        self.cmd_header
            .set_cfl(mem::size_of::<fis::RegH2D>() / mem::size_of::<u32>());
    }

    fn identify(&mut self) {
        let fis = fis::RegH2D::new()
            .with_command(ATACommand::Identify)
            .with_cflag()
            .with_count(1);
        self.setup_read_cmd(fis);
        self.cmd_table.set_prd(&mut self.identity);
        self.issue_synch();
        self.cmd_header.clear();
        self.eoi();
    }

    fn read_block(&mut self, data: &mut arch::Page, offset: u64) {
        let fis = fis::RegH2D::new()
            .with_command(ATACommand::ReadDMAExt)
            .with_cflag()
            .with_lba(offset / SECTOR_SIZE as u64)
            .with_device_lba()
            .with_count((fs::BSIZE / SECTOR_SIZE) as u16);
        self.setup_read_cmd(fis);
        self.cmd_table.set_prd(data.as_mut());
        self.issue();
    }

    fn write_block(&mut self, data: &arch::Page, offset: u64) {
        let fis = fis::RegH2D::new()
            .with_command(ATACommand::WriteDMAExt)
            .with_cflag()
            .with_lba(offset / SECTOR_SIZE as u64)
            .with_device_lba() // XXX: Why must we set this for write?
            .with_count(u16::try_from(fs::BSIZE / SECTOR_SIZE).unwrap());
        self.setup_write_cmd(fis);
        self.cmd_table.set_prd(data.as_slice());
        self.issue();
    }

    fn issue(&mut self) {
        let port = unsafe { &mut *self.port };
        port.wait();
        port.issue();
    }

    fn issue_synch(&mut self) {
        let port = unsafe { &mut *self.port };
        port.wait();
        port.issue();
        port.wait_done();
    }

    fn eoi(&mut self) {
        let port = unsafe { &mut *self.port };
        port.eoi();
        let ctlr = unsafe { &mut *self.ctlr };
        ctlr.eoi();
    }
}

pub const INTR_SD0: u32 = 14;

static DRIVE: Mutex<Option<&'static mut Drive>> = Mutex::new("drive", None);

pub unsafe fn init(mut conf: pci::Conf, abar: u64) {
    pci::setup_msi(&mut conf, 0, INTR_SD0);
    // unsafe {
    //     crate::ioapic::enable(INTR_SD0, 0);
    // }
    let ctl = unsafe { kmem::phys_to_mut::<GenericHostCtl>(abar) };
    let ctlp = ctl as *mut GenericHostCtl;
    let mut ghc = GlobalHBACtl::from_bits_truncate(volatile::read(&ctl.ghc));
    ghc |= GlobalHBACtl::AHCI_ENABLE;
    volatile::write(&mut ctl.ghc, ghc.bits());
    let pi = volatile::read(&ctl.ports_impl);
    for k in 0..32 {
        if pi & (1 << k) == 0 {
            continue;
        }
        let port = &mut ctl.port[k];
        if !port.is_present() || !port.is_storage() {
            continue;
        }
        let mut drive = DRIVE.lock();
        *drive = Some(Drive::new(port, ctlp));
        break;
    }
    ghc = GlobalHBACtl::from_bits_truncate(volatile::read(&ctl.ghc));
    ghc |= GlobalHBACtl::INTR_ENABLE;
    volatile::write(&mut ctl.ghc, ghc.bits());
}

static QUEUE: Mutex<Option<&bio::Buf>> = Mutex::new("diskqueue", None);

pub fn rdwr(buf: &'static bio::Buf) {
    assert!(buf.is_locked(), "sd::rdwr: buf not locked");
    assert_ne!(buf.flags(), bio::BufFlags::VALID, "sd::rdwr: nothing to do");

    let mut queue = QUEUE.lock();
    if queue.is_none() {
        start(buf);
    }
    *queue = bio::enqueue(queue.take(), buf);

    while buf.flags() & (bio::BufFlags::VALID | bio::BufFlags::DIRTY) != bio::BufFlags::VALID {
        crate::proc::myproc().sleep(buf.as_chan(), &QUEUE);
    }
}

fn start(buf: &bio::Buf) {
    let offset = buf.blockno() * fs::BSIZE as u64;
    let mut drive = DRIVE.lock();
    let Some(drive) = drive.as_deref_mut() else {
        panic!("no drive");
    };
    if buf.flags().contains(bio::BufFlags::DIRTY) {
        drive.write_block(buf.data_page(), offset);
    } else {
        drive.read_block(buf.data_page_mut(), offset);
    }
}

pub fn interrupt() {
    let mut queue = QUEUE.lock();
    let Some((buf, head)) = bio::dequeue(queue.take()) else {
        return;
    };
    *queue = head;
    buf.set_flags(bio::BufFlags::VALID);
    crate::proc::wakeup(buf.as_chan());
    if let Some(buf) = head {
        start(buf);
    }
    let mut drive = DRIVE.lock();
    if let Some(drive) = drive.as_deref_mut() {
        drive.eoi();
    } else {
        panic!("spurious drive interrupt");
    }
    unsafe {
        xapic::eoi();
    }
}
