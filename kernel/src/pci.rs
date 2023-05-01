use crate::acpi;
use crate::arch;
use crate::kmem;
use crate::sd;
use crate::trap;
use crate::vm;
use bitflags::bitflags;
use core::ptr;

pub type Bus = u8;

#[derive(Clone, Copy, Debug)]
pub enum Device {
    D0 = 0,
    D1 = 1,
    D2 = 2,
    D3 = 3,
    D4 = 4,
    D5 = 5,
    D6 = 6,
    D7 = 7,
    D8 = 8,
    D9 = 9,
    D10 = 10,
    D11 = 11,
    D12 = 12,
    D13 = 13,
    D14 = 14,
    D15 = 15,
    D16 = 16,
    D17 = 17,
    D18 = 18,
    D19 = 19,
    D20 = 20,
    D21 = 21,
    D22 = 22,
    D23 = 23,
    D24 = 24,
    D25 = 25,
    D26 = 26,
    D27 = 27,
    D28 = 28,
    D29 = 29,
    D30 = 30,
    D31 = 31,
}

static DEVICES: [Device; 32] = [
    Device::D0,
    Device::D1,
    Device::D2,
    Device::D3,
    Device::D4,
    Device::D5,
    Device::D6,
    Device::D7,
    Device::D8,
    Device::D9,
    Device::D10,
    Device::D11,
    Device::D12,
    Device::D13,
    Device::D14,
    Device::D15,
    Device::D16,
    Device::D17,
    Device::D18,
    Device::D19,
    Device::D20,
    Device::D21,
    Device::D22,
    Device::D23,
    Device::D24,
    Device::D25,
    Device::D26,
    Device::D27,
    Device::D28,
    Device::D29,
    Device::D30,
    Device::D31,
];

static FUNCTIONS: [Function; 8] = [
    Function::F0,
    Function::F1,
    Function::F2,
    Function::F3,
    Function::F4,
    Function::F5,
    Function::F6,
    Function::F7,
];

#[derive(Clone, Copy, Debug)]
pub enum Function {
    F0 = 0,
    F1 = 1,
    F2 = 2,
    F3 = 3,
    F4 = 4,
    F5 = 5,
    F6 = 6,
    F7 = 7,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    phys_addr: u64,
    _segment_group: u16,
    start_bus: Bus,
    end_bus: Bus,
}

impl Config {
    pub const fn empty() -> Config {
        Config {
            phys_addr: 0,
            _segment_group: 0,
            start_bus: 0,
            end_bus: 0,
        }
    }

    pub fn new(phys_addr: u64, _segment_group: u16, start_bus: u8, end_bus: u8) -> Config {
        Config {
            phys_addr,
            _segment_group,
            start_bus,
            end_bus,
        }
    }

    pub fn func_addr(&self, bus: Bus, device: Device, function: Function) -> u64 {
        (self.phys_addr + (u64::from(bus - self.start_bus) << 20))
            | (device as u64) << 15
            | (function as u64) << 12
    }
}

pub struct Conf {
    base: usize,
}

impl Conf {
    const COMMAND_OFF: usize = 4;

    pub fn new(base: usize) -> Self {
        assert_eq!(base % arch::PAGE_SIZE, 0);
        Self { base }
    }

    pub fn addr(&self, offset: usize) -> usize {
        assert!(offset < arch::PAGE_SIZE);
        self.base + offset
    }

    pub fn read<T>(&self, offset: usize) -> T {
        let addr = ptr::from_exposed_addr::<T>(self.addr(offset));
        unsafe { ptr::read_volatile(addr) }
    }

    pub fn write<T>(&self, offset: usize, val: T) {
        let addr = ptr::from_exposed_addr_mut::<T>(self.addr(offset));
        unsafe {
            ptr::write_volatile(addr, val);
        }
    }

    pub fn enable_bus_master(&mut self) {
        const BUS_MASTER_EN: u16 = 1 << 2;
        let cmd = self.read::<u16>(Self::COMMAND_OFF);
        self.write(Self::COMMAND_OFF, cmd | BUS_MASTER_EN);
    }

    pub fn enable_mem(&mut self) {
        const MEM_EN: u16 = 1 << 1;
        let cmd = self.read::<u16>(Self::COMMAND_OFF);
        self.write(Self::COMMAND_OFF, cmd | MEM_EN);
    }

    pub fn disable_intr(&mut self) {
        const INTR_DISABLE: u16 = 1 << 10;
        let cmd = self.read::<u16>(Self::COMMAND_OFF);
        self.write(Self::COMMAND_OFF, cmd | INTR_DISABLE);
    }

    pub fn find_msi_cap(&self) -> Option<usize> {
        const CAP_MIS: u8 = 0x05;
        let status = Status::from_bits_truncate(self.read(6));
        if !status.contains(Status::CAP_LIST) {
            panic!("no cap list for MSI");
        }
        let mut ptr = usize::from(self.read::<u8>(0x34));
        while ptr != 0 {
            let typ = self.read::<u8>(ptr);
            if typ == CAP_MIS {
                return Some(ptr);
            }
            ptr = usize::from(self.read::<u8>(ptr + 1));
        }
        None
    }
}

fn mapabar(kpage_table: &mut vm::PageTable, phys_addr: u64) -> Option<u32> {
    let addr = unsafe { kmem::phys_to_mut::<u32>(phys_addr) };
    let bar = unsafe { ptr::read_volatile(addr) };
    if bar == 0 || bar & 0b1 == 1 {
        return None;
    }
    let bits = unsafe {
        ptr::write_volatile(addr, !0);
        let bits = ptr::read_volatile(addr);
        ptr::write_volatile(addr, bar);
        bits
    };
    let size = !(bits & !0xF) + 1;
    kpage_table
        .map_phys_dev_range(bar as u64, (bar + size) as u64)
        .expect("mapped a BAR");
    crate::println!("abar at {phys_addr:x}, bar = {bar:x}, bits = {bits:08x}, size = {size}",);
    Some(bar)
}

const CLASS_STORAGE: u8 = 1;
const SUBCLASS_SATA: u8 = 6;
const PROG_IF_AHCI: u8 = 1;

const VENDOR_INTEL: u16 = 0x8086;
const DEVICE_SATA: u16 = 0x2922;

pub fn init(kpage_table: &mut vm::PageTable) {
    let configs = acpi::pci_configs();
    for config in configs.iter() {
        crate::println!("scanning PCIe {:x?}", config);
        for bus in config.start_bus..config.end_bus {
            for &dev in DEVICES.iter() {
                for &func in FUNCTIONS.iter() {
                    let phys_addr = config.func_addr(bus, dev, func);
                    let addr = kmem::phys_to_addr(phys_addr);
                    let mut conf = Conf::new(addr);
                    let vendor_id = conf.read::<u16>(0);
                    if vendor_id == 0xFFFF {
                        break;
                    }
                    let device_id = conf.read::<u16>(2);
                    let class_rev = conf.read::<u32>(8);
                    let class = (class_rev >> 24) as u8;
                    let subclass = (class_rev >> 16) as u8;
                    let prog_if = (class_rev >> 8) as u8;
                    let rev = class_rev as u8;
                    let typ = conf.read::<u8>(12 + 2);
                    let typ = typ & 0b0111_1111;
                    crate::print!("bus {bus}, {dev:?}, {func:?} at {phys_addr:x} ");
                    crate::print!("({vendor_id:x}/{device_id:x} - type {typ} class {class:x} ");
                    crate::println!("subclass {subclass:x} prog if {prog_if:x} rev {rev})");
                    if typ != 0 {
                        break;
                    }
                    if vendor_id == VENDOR_INTEL
                        && device_id == DEVICE_SATA
                        && class == CLASS_STORAGE
                        && subclass == SUBCLASS_SATA
                        && prog_if == PROG_IF_AHCI
                    {
                        conf.enable_mem();
                        const ABAR_OFFSET: u64 = 0x24;
                        unsafe {
                            sd::init(
                                conf,
                                mapabar(kpage_table, phys_addr + ABAR_OFFSET)
                                    .unwrap()
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
    }
}

// struct MSICapability {
//     cap_type: u8,
//     next: u8,
//     msg_ctl: u16,
//     msg_addr_lo: u32,
//     msg_addr_hi: u32,
//     data: u16,
// }

bitflags! {
    pub struct Status: u16 {
        const CAP_LIST = 1 << 4;
    }
}

pub fn setup_msi(conf: &mut Conf, cpu: u32, intr: u32) {
    const MSI_CTL_64BIT: u16 = 1 << 7;
    const MSI_CTL_EN: u16 = 1;
    let ptr = conf.find_msi_cap().expect("found MSI capability");
    let ctl = conf.read::<u16>(ptr + 2);
    if ctl & MSI_CTL_64BIT == 0 {
        panic!("MSI capability is not 64-bit capable");
    }
    conf.enable_bus_master();
    conf.disable_intr();
    let intr = trap::INTR0 + intr;
    let data = intr as u16 & 0xFF;
    let addr = 0x0FEE << 20 | (cpu & 0xFF);
    conf.write(ptr + 4, addr);
    conf.write(ptr + 12, data);
    conf.write(ptr + 2, ctl | MSI_CTL_EN);
}
