//! A reader for LLVM's stack maps, which say where a compiled program's garbage-collected pointers are at each call (D45).
//!
//! A program needs this while it runs, and must not have to link LLVM for it, so this crate has no LLVM and no shim (D46). Looking
//! a return address up allocates nothing (D24). Bad bytes are an error, never a panic (D3), and the bounds checks are proven (D22).
//!
//! ```
//! use lyftar_stack_maps::{Root, StackMaps, loaded_sections};
//!
//! # fn run() -> Result<(), lyftar_stack_maps::StackMapError> {
//! let mut stack_maps = StackMaps::new();
//! for section in loaded_sections()? {
//!     stack_maps.add_section(section)?;
//! }
//! // During a collection, for each return address on the stack:
//! # let return_address = 0;
//! if let Some(call) = stack_maps.find(return_address) {
//!     for root in call.record.statepoint()?.roots() {
//!         match root {
//!             Root::Pair { base, derived } => { /* move the object at `base`, and rewrite both */ }
//!             Root::Allocation(location) => { /* the pointers in this stack allocation */ }
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![warn(clippy::pedantic, missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod checks;

use std::fmt;
use std::ops::Range;

/// The format version this reads.
const VERSION: u8 = 3;
/// A location record's size in bytes.
const LOCATION_SIZE_IN_BYTES: usize = 12;
/// A live-out record's size in bytes.
const LIVE_OUT_SIZE_IN_BYTES: usize = 4;
/// A function record's size in bytes.
const FUNCTION_SIZE_IN_BYTES: usize = 24;
/// A constant's size in bytes.
const CONSTANT_SIZE_IN_BYTES: usize = 8;
/// A stack map header's size in bytes.
const HEADER_SIZE_IN_BYTES: usize = 16;
/// How many locations come first in a statepoint's record, ahead of what is live: its calling convention, its flags, and how many
/// deoptimization locations follow.
const STATEPOINT_HEADER_LOCATIONS: usize = 3;

/// The section's name in an ELF file.
const ELF_SECTION: &[u8] = b".llvm_stackmaps";
/// The section's segment in a Mach-O file.
const MACH_O_SEGMENT: &[u8] = b"__LLVM_STACKMAPS";
/// The section's name in a Mach-O file.
const MACH_O_SECTION: &[u8] = b"__llvm_stackmaps";

/// What is wrong with some stack map bytes, or with a file said to hold them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackMapError {
    /// What is wrong.
    pub problem: &'static str,
    /// How many bytes in it was found.
    pub offset_in_bytes: usize,
}

impl fmt::Display for StackMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}, at byte {}", self.problem, self.offset_in_bytes)
    }
}

impl std::error::Error for StackMapError {}

fn error<T>(problem: &'static str, offset_in_bytes: usize) -> Result<T, StackMapError> {
    Err(StackMapError { problem, offset_in_bytes })
}

/// Reads little-endian numbers out of bytes, where anything past the end is an error.
#[derive(Clone, Copy)]
struct Reader<'bytes> {
    bytes: &'bytes [u8],
}

impl<'bytes> Reader<'bytes> {
    fn slice(self, offset: usize, size: usize) -> Result<&'bytes [u8], StackMapError> {
        if !checks::range_is_inside_total(offset, size, self.bytes.len()) {
            return error("this runs past the end", offset);
        }
        match self.bytes.get(offset..offset + size) {
            Some(found) => Ok(found),
            None => error("this runs past the end", offset),
        }
    }

    fn array<const N: usize>(self, offset: usize) -> Result<[u8; N], StackMapError> {
        match <[u8; N]>::try_from(self.slice(offset, N)?) {
            Ok(found) => Ok(found),
            Err(_) => error("this runs past the end", offset),
        }
    }

    fn u8(self, offset: usize) -> Result<u8, StackMapError> {
        Ok(self.array::<1>(offset)?[0])
    }

    fn u16(self, offset: usize) -> Result<u16, StackMapError> {
        Ok(u16::from_le_bytes(self.array(offset)?))
    }

    fn u32(self, offset: usize) -> Result<u32, StackMapError> {
        Ok(u32::from_le_bytes(self.array(offset)?))
    }

    fn i32(self, offset: usize) -> Result<i32, StackMapError> {
        Ok(i32::from_le_bytes(self.array(offset)?))
    }

    fn u64(self, offset: usize) -> Result<u64, StackMapError> {
        Ok(u64::from_le_bytes(self.array(offset)?))
    }
}

/// Where a value is at a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    /// In a register, by its DWARF number.
    Register {
        /// The register's DWARF number.
        register: u16,
        /// The value's size in bytes.
        size_in_bytes: u16,
    },
    /// The value is a register's content plus an offset: the address of a stack allocation.
    Direct {
        /// The register's DWARF number.
        register: u16,
        /// What is added to it.
        offset_in_bytes: i32,
        /// The value's size in bytes.
        size_in_bytes: u16,
    },
    /// In memory, at a register's content plus an offset: a stack slot.
    Indirect {
        /// The register's DWARF number.
        register: u16,
        /// What is added to it.
        offset_in_bytes: i32,
        /// The value's size in bytes.
        size_in_bytes: u16,
    },
    /// A constant, in the record itself.
    Constant {
        /// The constant.
        value: i32,
        /// The value's size in bytes.
        size_in_bytes: u16,
    },
    /// A constant from the stack map's list of them.
    LargeConstant {
        /// Where it is in the list.
        index: u32,
        /// The constant.
        value: u64,
        /// The value's size in bytes.
        size_in_bytes: u16,
    },
}

/// What a function in a stack map says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionEntry {
    /// The function's address. In an object file nothing has relocated this yet, so it is usually 0.
    pub address: u64,
    /// The function's frame, in bytes, or `u64::MAX` if it has stack allocations of a size only known when it runs.
    pub stack_size_in_bytes: u64,
    /// How many records in a row belong to this function.
    pub record_count: u64,
}

/// A stack map record: one call, one patchpoint or one stack map intrinsic.
#[derive(Clone, Copy, Debug)]
pub struct Record<'table, 'bytes> {
    table: &'table Table<'bytes>,
    offset: usize,
    location_count: u16,
    live_out_count: u16,
}

impl<'table, 'bytes> Record<'table, 'bytes> {
    fn reader(self) -> Reader<'bytes> {
        Reader { bytes: self.table.bytes }
    }

    /// The id the statepoint or patchpoint gave.
    #[must_use]
    pub fn id(self) -> u64 {
        // A table only exists once each of these has been read once.
        self.reader().u64(self.offset).unwrap_or(0)
    }

    /// How far into its function the call returns to.
    #[must_use]
    pub fn instruction_offset_in_bytes(self) -> u32 {
        self.reader().u32(self.offset + 8).unwrap_or(0)
    }

    /// How many locations this has.
    #[must_use]
    pub fn location_count(self) -> u16 {
        self.location_count
    }

    /// The location at this index, if it has one.
    #[must_use]
    pub fn location(self, index: u16) -> Option<Location> {
        if !checks::index_is_inside(u32::from(index), u32::from(self.location_count)) {
            return None;
        }
        self.table.location_at(self.offset + 16 + usize::from(index) * LOCATION_SIZE_IN_BYTES).ok()
    }

    /// Each of its locations, in order.
    #[must_use]
    pub fn locations(self) -> impl ExactSizeIterator<Item = Location> + 'table {
        let record = self;
        (0..self.location_count)
            .map(move |index| record.location(index).unwrap_or(Location::Constant { value: 0, size_in_bytes: 0 }))
    }

    /// The registers that are live after the call, each as its DWARF number and size in bytes. Only patchpoints have these.
    #[must_use]
    pub fn live_outs(self) -> impl ExactSizeIterator<Item = (u16, u8)> + 'table {
        let locations_end = self.offset + checks::locations_end_in_record(self.location_count);
        let start = checks::align_to_8(locations_end).unwrap_or(locations_end) + 4;
        let reader = self.reader();
        (0..usize::from(self.live_out_count)).map(move |index| {
            let offset = start + index * LIVE_OUT_SIZE_IN_BYTES;
            (reader.u16(offset).unwrap_or(0), reader.u8(offset + 3).unwrap_or(0))
        })
    }

    /// This record read as a statepoint's: LLVM's three constants, deoptimization locations, then what is live.
    ///
    /// # Errors
    /// If it doesn't have that shape, as a stack map intrinsic's record usually doesn't.
    pub fn statepoint(self) -> Result<Statepoint<'table, 'bytes>, StackMapError> {
        let shape = "this record isn't a statepoint's";
        let constant = |index: u16| match self.location(index) {
            Some(Location::Constant { value, .. }) => Ok(value),
            _ => error(shape, self.offset),
        };
        let calling_convention = constant(0)?;
        let flags = constant(1)?;
        let deoptimization_count = constant(2)?;
        let Ok(deoptimization_count) = u16::try_from(deoptimization_count) else {
            return error(shape, self.offset);
        };
        let live_start = usize::from(deoptimization_count) + STATEPOINT_HEADER_LOCATIONS;
        if live_start > usize::from(self.location_count) {
            return error(shape, self.offset);
        }
        let statepoint = Statepoint { record: self, calling_convention, flags, deoptimization_count };
        // A pair needs both its locations, and only a stack allocation stands alone.
        let mut roots = statepoint.roots();
        while roots.next().is_some() {}
        if roots.is_broken {
            return error(shape, self.offset);
        }
        Ok(statepoint)
    }
}

/// A statepoint's record, read as what it says.
#[derive(Clone, Copy, Debug)]
pub struct Statepoint<'table, 'bytes> {
    record: Record<'table, 'bytes>,
    /// The calling convention of what it calls.
    pub calling_convention: i32,
    /// The flags it was given.
    pub flags: i32,
    deoptimization_count: u16,
}

impl<'table, 'bytes> Statepoint<'table, 'bytes> {
    /// How many deoptimization locations it has.
    #[must_use]
    pub fn deoptimization_count(self) -> u16 {
        self.deoptimization_count
    }

    /// The deoptimization locations.
    #[must_use]
    pub fn deoptimization(self) -> impl ExactSizeIterator<Item = Location> + 'table {
        let record = self.record;
        (0..self.deoptimization_count)
            .map(move |index| record.location(index + 3).unwrap_or(Location::Constant { value: 0, size_in_bytes: 0 }))
    }

    /// What is live across the call, each one once.
    #[must_use]
    pub fn roots(self) -> Roots<'table, 'bytes> {
        Roots { record: self.record, next: self.deoptimization_count + 3, is_broken: false }
    }
}

/// A place a collector has to look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// A pointer into an object, and the start of that object. When they are the same value, they are at the same place or hold
    /// the same bits. After moving the object, write the new start at `base`, and at `derived` the new start plus what `derived`
    /// was past the old one.
    Pair {
        /// Where the start of the object is.
        base: Location,
        /// Where the pointer that may point inside the object is.
        derived: Location,
    },
    /// A stack allocation holding pointers, from a statepoint's `gc-live` list. The value is the allocation's address.
    Allocation(Location),
}

/// An iterator over a statepoint's roots. It allocates nothing.
#[derive(Clone, Debug)]
pub struct Roots<'table, 'bytes> {
    record: Record<'table, 'bytes>,
    next: u16,
    is_broken: bool,
}

impl Iterator for Roots<'_, '_> {
    type Item = Root;

    fn next(&mut self) -> Option<Root> {
        let first = self.record.location(self.next)?;
        // LLVM lists stack allocations after the pairs, and they are the only location of this kind that are live.
        if matches!(first, Location::Direct { .. }) {
            self.next += 1;
            return Some(Root::Allocation(first));
        }
        let Some(second) = self.next.checked_add(1).and_then(|index| self.record.location(index)) else {
            self.is_broken = true;
            self.next = self.record.location_count;
            return None;
        };
        self.next += 2;
        Some(Root::Pair { base: first, derived: second })
    }
}

/// One translation unit's stack map, with every bound checked when it was read.
#[derive(Clone, Copy, Debug)]
pub struct Table<'bytes> {
    bytes: &'bytes [u8],
    function_count: u32,
    constant_count: u32,
    record_count: u32,
    records_offset: usize,
    size_in_bytes: usize,
}

impl<'bytes> Table<'bytes> {
    /// Reads the table at the start of these bytes, which can go on past it.
    ///
    /// # Errors
    /// If it isn't version 3 of the format, or anything in it runs past the end or doesn't add up.
    pub fn parse(bytes: &'bytes [u8]) -> Result<Table<'bytes>, StackMapError> {
        let reader = Reader { bytes };
        if !checks::range_is_inside_total(0, HEADER_SIZE_IN_BYTES, bytes.len()) {
            return error("a stack map starts with a 16-byte header", 0);
        }
        if reader.u8(0)? != VERSION {
            return error("this isn't version 3 of the stack map format", 0);
        }
        let function_count = reader.u32(4)?;
        let constant_count = reader.u32(8)?;
        let record_count = reader.u32(12)?;
        let Ok(records_offset) = usize::try_from(checks::lists_size_in_bytes(function_count, constant_count)) else {
            return error("the lists are too long for this machine", 4);
        };
        if !checks::range_is_inside_total(0, records_offset, bytes.len()) {
            return error("the function and constant lists run past the end", HEADER_SIZE_IN_BYTES);
        }
        let mut table = Table { bytes, function_count, constant_count, record_count, records_offset, size_in_bytes: 0 };
        let mut records_said = 0u64;
        for index in 0..function_count {
            let more = table.function(index)?.record_count;
            if !checks::record_count_fits(records_said, more, u64::from(record_count)) {
                return error("the functions say more records than the header", HEADER_SIZE_IN_BYTES);
            }
            records_said += more;
        }
        if records_said != u64::from(record_count) {
            return error("the functions say fewer records than the header", HEADER_SIZE_IN_BYTES);
        }
        let mut offset = records_offset;
        for _ in 0..record_count {
            offset = table.check_record(offset)?;
        }
        table.size_in_bytes = offset;
        Ok(table)
    }

    /// Checks the record here, with each location in it, and says where the next one starts.
    fn check_record(&self, offset: usize) -> Result<usize, StackMapError> {
        let (location_count, _, end) = self.record_shape(offset)?;
        let first = offset + 16;
        for index in 0..usize::from(location_count) {
            self.location_at(first + index * LOCATION_SIZE_IN_BYTES)?;
        }
        Ok(end)
    }

    /// A record's location count, live-out count and end, with its whole length checked against the bytes.
    fn record_shape(&self, offset: usize) -> Result<(u16, u16, usize), StackMapError> {
        let reader = Reader { bytes: self.bytes };
        let total = self.bytes.len();
        let location_count = reader.u16(offset + 14)?;
        let locations_size = checks::locations_end_in_record(location_count);
        if !checks::range_is_inside_total(offset, locations_size, total) {
            return error("a record's locations run past the end", offset);
        }
        let Some(live_outs_start) = checks::align_to_8(offset + locations_size) else {
            return error("a record's locations run past the end", offset);
        };
        if !checks::range_is_inside_total(live_outs_start, 4, total) {
            return error("a record's live-out count runs past the end", live_outs_start);
        }
        let live_out_count = reader.u16(live_outs_start + 2)?;
        let live_outs_size = checks::live_outs_size_in_bytes(live_out_count);
        if !checks::range_is_inside_total(live_outs_start, live_outs_size, total) {
            return error("a record's live-outs run past the end", live_outs_start);
        }
        let Some(end) = checks::align_to_8(live_outs_start + live_outs_size) else {
            return error("a record's live-outs run past the end", live_outs_start);
        };
        if end > total {
            return error("a record's padding runs past the end", live_outs_start);
        }
        Ok((location_count, live_out_count, end))
    }

    fn location_at(&self, offset: usize) -> Result<Location, StackMapError> {
        let reader = Reader { bytes: self.bytes };
        let kind = reader.u8(offset)?;
        let size_in_bytes = reader.u16(offset + 2)?;
        let register = reader.u16(offset + 4)?;
        let value = reader.i32(offset + 8)?;
        Ok(match kind {
            1 => Location::Register { register, size_in_bytes },
            2 => Location::Direct { register, offset_in_bytes: value, size_in_bytes },
            3 => Location::Indirect { register, offset_in_bytes: value, size_in_bytes },
            4 => Location::Constant { value, size_in_bytes },
            5 => {
                let Ok(index) = u32::try_from(value) else {
                    return error("a large constant's index is negative", offset);
                };
                if !checks::index_is_inside(index, self.constant_count) {
                    return error("a large constant's index is past the list", offset);
                }
                Location::LargeConstant { index, value: self.constant(index)?, size_in_bytes }
            }
            _ => return error("a location has a kind this format doesn't have", offset),
        })
    }

    /// How many bytes this table takes.
    #[must_use]
    pub fn size_in_bytes(&self) -> usize {
        self.size_in_bytes
    }

    /// How many functions it has.
    #[must_use]
    pub fn function_count(&self) -> u32 {
        self.function_count
    }

    /// How many large constants it has.
    #[must_use]
    pub fn constant_count(&self) -> u32 {
        self.constant_count
    }

    /// How many records it has.
    #[must_use]
    pub fn record_count(&self) -> u32 {
        self.record_count
    }

    /// The function at this index.
    ///
    /// # Errors
    /// If it has no function at that index.
    pub fn function(&self, index: u32) -> Result<FunctionEntry, StackMapError> {
        if !checks::index_is_inside(index, self.function_count) {
            return error("this table has no function at that index", HEADER_SIZE_IN_BYTES);
        }
        let reader = Reader { bytes: self.bytes };
        let offset = HEADER_SIZE_IN_BYTES + index as usize * FUNCTION_SIZE_IN_BYTES;
        Ok(FunctionEntry {
            address: reader.u64(offset)?,
            stack_size_in_bytes: reader.u64(offset + 8)?,
            record_count: reader.u64(offset + 16)?,
        })
    }

    /// The large constant at this index.
    ///
    /// # Errors
    /// If it has no constant at that index.
    pub fn constant(&self, index: u32) -> Result<u64, StackMapError> {
        if !checks::index_is_inside(index, self.constant_count) {
            return error("this table has no constant at that index", HEADER_SIZE_IN_BYTES);
        }
        let reader = Reader { bytes: self.bytes };
        reader.u64(
            HEADER_SIZE_IN_BYTES
                + self.function_count as usize * FUNCTION_SIZE_IN_BYTES
                + index as usize * CONSTANT_SIZE_IN_BYTES,
        )
    }

    /// Each record, in order.
    #[must_use]
    pub fn records(&self) -> Records<'_, 'bytes> {
        Records { table: self, offset: self.records_offset, remaining: self.record_count }
    }
}

/// An iterator over a table's records. It allocates nothing.
#[derive(Clone, Debug)]
pub struct Records<'table, 'bytes> {
    table: &'table Table<'bytes>,
    offset: usize,
    remaining: u32,
}

impl<'table, 'bytes> Iterator for Records<'table, 'bytes> {
    type Item = Record<'table, 'bytes>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        // `Table::parse` checked each of these.
        let (location_count, live_out_count, end) = self.table.record_shape(self.offset).ok()?;
        let record = Record { table: self.table, offset: self.offset, location_count, live_out_count };
        self.offset = end;
        self.remaining -= 1;
        Some(record)
    }
}

/// Each table in a section. A linked program's section is one table per translation unit, one after another.
///
/// # Errors
/// If any of them is malformed.
pub fn parse_section(section: &[u8]) -> Result<Vec<Table<'_>>, StackMapError> {
    let mut tables = Vec::new();
    let mut offset = 0;
    while offset < section.len() {
        // Linkers can pad between what they join.
        let Some(rest) = section.get(offset..) else {
            return error("this runs past the end", offset);
        };
        if rest.iter().take(8).all(|byte| *byte == 0) {
            offset += rest.len().min(8);
            continue;
        }
        match Table::parse(rest) {
            Ok(table) => {
                offset += table.size_in_bytes();
                tables.push(table);
            }
            Err(found) => {
                return Err(StackMapError { problem: found.problem, offset_in_bytes: offset + found.offset_in_bytes });
            }
        }
    }
    Ok(tables)
}

/// A call found in a [`StackMaps`], by the address it returns to.
#[derive(Clone, Copy, Debug)]
pub struct CallSite<'table, 'bytes> {
    /// The record for the call.
    pub record: Record<'table, 'bytes>,
    /// The function it is in.
    pub function: FunctionEntry,
}

/// One entry in a [`StackMaps`]' index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexEntry {
    return_address: u64,
    table: u32,
    function: u32,
    record_offset: usize,
}

/// The stack maps of a whole program, from any number of translation units, indexed by the address each call returns to.
/// Building one allocates, and [`find`](Self::find) doesn't, so a collector can use it during a collection.
#[derive(Clone, Debug, Default)]
pub struct StackMaps<'bytes> {
    tables: Vec<Table<'bytes>>,
    index: Vec<IndexEntry>,
}

impl<'bytes> StackMaps<'bytes> {
    /// An empty one.
    #[must_use]
    pub fn new() -> Self {
        StackMaps { tables: Vec::new(), index: Vec::new() }
    }

    /// Adds each table in a section whose function addresses have been relocated, as in a loaded program or a JIT's memory. A
    /// function at address 0 has been stripped by a linker, and is left out.
    ///
    /// # Errors
    /// If it is malformed, or says two calls return to the same address.
    pub fn add_section(&mut self, section: &'bytes [u8]) -> Result<(), StackMapError> {
        for table in parse_section(section)? {
            let Ok(table_index) = u32::try_from(self.tables.len()) else {
                return error("this has too many tables", 0);
            };
            self.tables.push(table);
            let mut records = table.records();
            for function_index in 0..table.function_count() {
                let function = table.function(function_index)?;
                for _ in 0..function.record_count {
                    let Some(record) = records.next() else {
                        return error("the functions say more records than the table has", 0);
                    };
                    if function.address == 0 {
                        continue;
                    }
                    let Some(return_address) =
                        function.address.checked_add(u64::from(record.instruction_offset_in_bytes()))
                    else {
                        return error("a record's return address overflows", record.offset);
                    };
                    self.index.push(IndexEntry {
                        return_address,
                        table: table_index,
                        function: function_index,
                        record_offset: record.offset,
                    });
                }
            }
        }
        self.index.sort_unstable_by_key(|entry| entry.return_address);
        if self.index.windows(2).any(|pair| pair[0].return_address == pair[1].return_address) {
            return error("two calls return to the same address", 0);
        }
        Ok(())
    }

    /// How many calls it knows about.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.index.len()
    }

    /// The tables it holds.
    #[must_use]
    pub fn tables(&self) -> &[Table<'bytes>] {
        &self.tables
    }

    /// The call that returns to this address, if there is one. This allocates nothing (D24).
    #[must_use]
    pub fn find(&self, return_address: u64) -> Option<CallSite<'_, 'bytes>> {
        let found = self.index.binary_search_by_key(&return_address, |entry| entry.return_address).ok()?;
        let entry = self.index.get(found)?;
        let table = self.tables.get(entry.table as usize)?;
        let (location_count, live_out_count, _) = table.record_shape(entry.record_offset).ok()?;
        Some(CallSite {
            record: Record { table, offset: entry.record_offset, location_count, live_out_count },
            function: table.function(entry.function).ok()?,
        })
    }
}

/// Finds the stack map section in the bytes of an object file or a linked program, 64-bit ELF or Mach-O. An object file's function
/// addresses haven't been relocated, so [`parse_section`] reads it, and [`StackMaps::add_section`] wants a loaded one.
///
/// # Errors
/// If this isn't a little-endian 64-bit ELF or Mach-O file, or its headers run past the end.
pub fn section_in_file(file: &[u8]) -> Result<Option<&[u8]>, StackMapError> {
    let reader = Reader { bytes: file };
    match file.get(..4) {
        Some([0x7f, b'E', b'L', b'F']) => elf_section(reader),
        Some([0xcf, 0xfa, 0xed, 0xfe]) => mach_o_section(reader),
        _ => error("this isn't a little-endian 64-bit ELF or Mach-O file", 0),
    }
}

/// The range an ELF section's bytes take in the file, with its address, by name.
fn elf_section_header(reader: Reader<'_>, wanted: &[u8]) -> Result<Option<(Range<usize>, u64)>, StackMapError> {
    if reader.u8(4)? != 2 || reader.u8(5)? != 1 {
        return error("this ELF file isn't little-endian and 64-bit", 4);
    }
    let as_size = |value: u64, offset: usize| match usize::try_from(value) {
        Ok(found) => Ok(found),
        Err(_) => error("this ELF file is too big for this machine", offset),
    };
    let headers_at = as_size(reader.u64(40)?, 40)?;
    let header_size = usize::from(reader.u16(58)?);
    let header_count = usize::from(reader.u16(60)?);
    let names_index = usize::from(reader.u16(62)?);
    if header_size < 64 {
        return error("this ELF file's section headers are too small", 58);
    }
    let header = |index: usize| headers_at.checked_add(index.checked_mul(header_size)?);
    let Some(names_header) = header(names_index) else {
        return error("this ELF file's section headers run past the end", 40);
    };
    let table_at = as_size(reader.u64(names_header.saturating_add(24))?, names_header)?;
    let table_size = as_size(reader.u64(names_header.saturating_add(32))?, names_header)?;
    let name_table = reader.slice(table_at, table_size)?;
    for index in 0..header_count {
        let Some(at) = header(index) else {
            return error("this ELF file's section headers run past the end", 40);
        };
        let name_at = reader.u32(at)? as usize;
        let name = name_table.get(name_at..).unwrap_or_default();
        let name = name.split(|byte| *byte == 0).next().unwrap_or_default();
        if name != wanted {
            continue;
        }
        let address = reader.u64(at.saturating_add(16))?;
        let offset = as_size(reader.u64(at.saturating_add(24))?, at)?;
        let size = as_size(reader.u64(at.saturating_add(32))?, at)?;
        reader.slice(offset, size)?;
        return Ok(Some((offset..offset + size, address)));
    }
    Ok(None)
}

fn elf_section(reader: Reader<'_>) -> Result<Option<&[u8]>, StackMapError> {
    Ok(elf_section_header(reader, ELF_SECTION)?.and_then(|(range, _)| reader.bytes.get(range)))
}

fn mach_o_section(reader: Reader<'_>) -> Result<Option<&[u8]>, StackMapError> {
    const LOAD_COMMANDS_AT: usize = 32;
    const SEGMENT_64: u32 = 0x19;
    const SEGMENT_HEADER_SIZE: usize = 72;
    const SECTION_HEADER_SIZE: usize = 80;
    let command_count = reader.u32(16)?;
    let mut command = LOAD_COMMANDS_AT;
    for _ in 0..command_count {
        let kind = reader.u32(command)?;
        let size = reader.u32(command.saturating_add(4))? as usize;
        if size == 0 {
            return error("a Mach-O load command says it has no size", command);
        }
        if kind == SEGMENT_64 {
            let section_count = reader.u32(command.saturating_add(64))? as usize;
            for index in 0..section_count {
                let at = command
                    .saturating_add(SEGMENT_HEADER_SIZE)
                    .saturating_add(index.saturating_mul(SECTION_HEADER_SIZE));
                let name = fixed_name(reader.slice(at, 16)?);
                let segment = fixed_name(reader.slice(at.saturating_add(16), 16)?);
                if name != MACH_O_SECTION || segment != MACH_O_SEGMENT {
                    continue;
                }
                let Ok(size_of_section) = usize::try_from(reader.u64(at.saturating_add(40))?) else {
                    return error("this Mach-O file is too big for this machine", at);
                };
                let offset = reader.u32(at.saturating_add(48))? as usize;
                return reader.slice(offset, size_of_section).map(Some);
            }
        }
        command = command.saturating_add(size);
    }
    Ok(None)
}

/// A name in a Mach-O header: 16 bytes, ending at a NUL if it is shorter.
fn fixed_name(bytes: &[u8]) -> &[u8] {
    bytes.split(|byte| *byte == 0).next().unwrap_or_default()
}

/// Each stack map section of what is loaded in this process: the program and any library it has loaded, relocated. A linker that
/// strips what nothing refers to drops these, so a program using stack maps can't be linked with `-dead_strip` or `--gc-sections`.
///
/// # Errors
/// If a file can't be read on a platform where that's how they are found, or its headers are bad.
#[cfg(target_os = "macos")]
pub fn loaded_sections() -> Result<Vec<&'static [u8]>, StackMapError> {
    use std::ffi::{c_char, c_ulong, c_void};
    unsafe extern "C" {
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_header(image_index: u32) -> *const c_void;
        fn getsectiondata(
            header: *const c_void,
            segment: *const c_char,
            section: *const c_char,
            size: *mut c_ulong,
        ) -> *const u8;
    }
    let mut found = Vec::new();
    // SAFETY: each is a plain query of dyld, with an index below its count. An image can go if something unloads it, which a
    // program with stack maps in a library has to avoid while using these.
    unsafe {
        for index in 0.._dyld_image_count() {
            let header = _dyld_get_image_header(index);
            if header.is_null() {
                continue;
            }
            let mut size: c_ulong = 0;
            let start =
                getsectiondata(header, c"__LLVM_STACKMAPS".as_ptr(), c"__llvm_stackmaps".as_ptr(), &raw mut size);
            if !start.is_null() {
                let Ok(length) = usize::try_from(size) else {
                    return error("a stack map section is too big for this machine", 0);
                };
                found.push(std::slice::from_raw_parts(start, length));
            }
        }
    }
    Ok(found)
}

/// Each stack map section of what is loaded in this process: the program and any library it has loaded, relocated. A linker that
/// strips what nothing refers to drops these, so a program using stack maps can't be linked with `--gc-sections`. The section is
/// read-only and holds addresses, so a position-independent program has to be linked with `-z notext`, or not be one.
///
/// # Errors
/// If a file can't be read, or its headers are bad.
#[cfg(target_os = "linux")]
pub fn loaded_sections() -> Result<Vec<&'static [u8]>, StackMapError> {
    use std::ffi::{CStr, OsStr, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;

    /// The start of glibc's and musl's `struct dl_phdr_info`, which is all this reads.
    #[repr(C)]
    struct LoadedObject {
        address: u64,
        name: *const c_char,
    }
    unsafe extern "C" {
        fn dl_iterate_phdr(
            callback: unsafe extern "C" fn(*const LoadedObject, usize, *mut c_void) -> c_int,
            data: *mut c_void,
        ) -> c_int;
    }
    unsafe extern "C" fn collect(object: *const LoadedObject, _size: usize, data: *mut c_void) -> c_int {
        // SAFETY: the C library gives a live object and the vector this was passed for the call. Nothing here panics into it:
        // a push that can't allocate aborts.
        unsafe {
            let objects = &mut *data.cast::<Vec<(u64, Vec<u8>)>>();
            let name = if (*object).name.is_null() { &[] } else { CStr::from_ptr((*object).name).to_bytes() };
            objects.push(((*object).address, name.to_vec()));
        }
        0
    }
    let mut objects: Vec<(u64, Vec<u8>)> = Vec::new();
    // SAFETY: `collect` only uses the vector, which outlives the call.
    unsafe { dl_iterate_phdr(collect, (&raw mut objects).cast()) };
    let mut sections = Vec::new();
    for (address, name) in objects {
        // The program itself has an empty name, and the kernel's vDSO has one with no file behind it.
        let path = if name.is_empty() { OsStr::new("/proc/self/exe") } else { OsStr::from_bytes(&name) };
        let Ok(file) = std::fs::read(path) else {
            continue;
        };
        let Some((range, section_address)) = elf_section_header(Reader { bytes: &file }, ELF_SECTION)? else {
            continue;
        };
        let Some(start) = address.checked_add(section_address).and_then(|start| usize::try_from(start).ok()) else {
            return error("a stack map section's address is too big for this machine", 0);
        };
        // SAFETY: the ELF file says this section is loaded at this offset from where the C library says the object is, for as long
        // as it stays loaded, which a program with stack maps in a library has to see to while using these.
        sections.push(unsafe { std::slice::from_raw_parts(start as *const u8, range.len()) });
    }
    Ok(sections)
}
