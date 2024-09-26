# The rxv64 Operating System

rxv64 is a pedagogical operating system written in Rust that targets
multiprocessor x86_64 machines.  It is a reimplementation of the xv6
operating system from MIT.

As a pedagogical system, it supports very little hardware other than
the text-mode CGA device, serial port, PS/2 keyboard controller, and
PCIe AHCI SATA storage devices.

See the xv6 README for more information on provenance and intended
use.

cd bin && gcc mkfs.c -o mkfs
sudo apt install clang llvm lld

cargo xtask build --release
cd cmd && sh mk
cargo xtask run