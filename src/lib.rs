#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{fmt, marker::PhantomData, mem::MaybeUninit};

use arrayvec::ArrayString;
use bytemuck::AnyBitPattern;

pub trait MemReader: Sized {
    fn read<T: AnyBitPattern>(&self, addr: u64) -> Option<T>;
}

pub trait Binding<T> {
    fn read(self, addr: u64) -> Option<T>;
}

pub trait Resolve: Sized {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self>;
}

#[repr(C)]
pub struct Pointer<T> {
    address: u64,
    _t: PhantomData<T>,
}

impl<T> Copy for Pointer<T> {}

impl<T> Clone for Pointer<T> {
    fn clone(&self) -> Self {
        Self {
            address: self.address.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Pointer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pointer")
            .field("address", &self.address)
            .field("type", &core::any::type_name::<T>())
            .finish()
    }
}

unsafe impl<T: 'static> ::bytemuck::AnyBitPattern for Pointer<T> {}
unsafe impl<T> ::bytemuck::Zeroable for Pointer<T> {}

impl<T: Resolve> Pointer<T> {
    pub fn resolve(self, reader: impl MemReader) -> Option<T> {
        if self.address == 0 {
            None
        } else {
            T::resolve(reader, self.address)
        }
    }
}

impl<T: AnyBitPattern> Pointer<T> {
    pub fn read(self, reader: impl MemReader) -> Option<T> {
        if self.address == 0 {
            None
        } else {
            reader.read(self.address)
        }
    }
}

impl<T> Pointer<T> {
    pub fn resolve_with<R: Binding<T>>(self, binding: R) -> Option<T> {
        self.resolve_as_with(binding)
    }

    pub fn resolve_as_with<U, R: Binding<U>>(self, binding: R) -> Option<U> {
        if self.address == 0 {
            None
        } else {
            binding.read(self.address)
        }
    }

    pub fn deref(self, reader: impl MemReader) -> Option<u64> {
        reader.read(self.address)
    }

    pub fn address_value(self) -> u64 {
        self.address
    }
}

pub struct Array<T> {
    addr: u64,
    size: u32,
    _t: PhantomData<T>,
}

impl<T> Copy for Array<T> {}

impl<T> Clone for Array<T> {
    fn clone(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            size: self.size.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Array<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Array")
            .field("addr", &self.addr)
            .field("size", &self.size)
            .field("type", &core::any::type_name::<T>())
            .finish()
    }
}

impl<T> Array<T> {
    const SIZE: u64 = 0x18;
    const DATA: u64 = 0x20;

    pub fn size(&self) -> u32 {
        self.size
    }
}

impl<T: AnyBitPattern> Array<T> {
    pub fn iter<R: MemReader>(self, reader: R) -> ArrayIter<T, R> {
        let start = self.addr + Self::DATA;
        let end = start + (core::mem::size_of::<T>() * self.size as usize) as u64;

        ArrayIter {
            pos: start,
            end,
            reader,
            _t: PhantomData,
        }
    }

    pub fn get<R: MemReader>(self, reader: R, index: usize) -> Option<T> {
        let offset = self.addr + Self::DATA + (index * core::mem::size_of::<T>()) as u64;
        reader.read(offset)
    }

    pub unsafe fn as_slice<R: MemReader>(&self, reader: R) -> Option<&[MaybeUninit<T>]> {
        let len = reader.read(self.addr + Self::SIZE)?;
        let data = (self.addr + Self::DATA) as usize as *const MaybeUninit<T>;

        Some(unsafe { ::core::slice::from_raw_parts(data, len) })
    }
}

impl<T> Resolve for Array<T> {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self> {
        let size = reader.read(addr + Self::SIZE)?;
        Some(Self {
            addr,
            size,
            _t: PhantomData,
        })
    }
}

pub struct ArrayIter<T, R> {
    pos: u64,
    end: u64,
    reader: R,
    _t: PhantomData<T>,
}

impl<T: AnyBitPattern, R: MemReader> Iterator for ArrayIter<T, R> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.end {
            return None;
        }

        let item: T = self.reader.read(self.pos)?;

        self.pos = self.pos + (core::mem::size_of::<T>() as u64);
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end.saturating_sub(self.pos) as usize;
        (remaining, Some(remaining))
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CSString {
    addr: u64,
    size: u32,
}

impl CSString {
    const SIZE: u64 = 0x10;
    const DATA: u64 = 0x14;

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn chars(self, reader: impl MemReader) -> impl Iterator<Item = char> {
        let start = self.addr + Self::DATA;
        let end = start + u64::from(2 * self.size);

        let utf16 = ArrayIter {
            pos: start,
            end,
            reader,
            _t: PhantomData::<u16>,
        };
        char::decode_utf16(utf16).map(|o| o.unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    pub fn to_string<const CAP: usize>(self, reader: impl MemReader) -> ArrayString<CAP> {
        let mut s = ArrayString::new();
        for c in self.chars(reader) {
            match s.try_push(c) {
                Ok(()) => {}
                Err(_) => break,
            }
        }
        s
    }

    #[cfg(feature = "alloc")]
    pub fn to_std_string(self, reader: impl MemReader) -> ::alloc::string::String {
        self.chars(reader).collect()
    }
}

impl Resolve for CSString {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self> {
        let size = reader.read(addr + Self::SIZE)?;
        Some(Self { addr, size })
    }
}

pub struct List<T> {
    addr: u64,
    items: Array<T>,
    size: u32,
}

impl<T> Copy for List<T> {}

impl<T> Clone for List<T> {
    fn clone(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            items: self.items.clone(),
            size: self.size.clone(),
        }
    }
}

impl<T> fmt::Debug for List<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("List")
            .field("addr", &self.addr)
            .field("items", &self.items)
            .field("size", &self.size)
            .finish()
    }
}

impl<T> List<T> {
    const ITEMS: u64 = 0x10;
    const SIZE: u64 = 0x18;

    pub fn size(&self) -> u32 {
        self.size
    }
}

impl<T: AnyBitPattern + 'static> List<T> {
    pub fn iter(self, reader: impl MemReader) -> impl Iterator<Item = T> {
        self.items.iter(reader).take(self.size as _)
    }

    pub fn get<R: MemReader>(self, reader: R, index: usize) -> Option<T> {
        self.items.get(reader, index)
    }

    pub unsafe fn as_slice<R: MemReader>(&self, reader: R) -> Option<&[T]> {
        let inner = unsafe { self.items.as_slice(reader)? };
        let inner = &inner[..self.size as usize];
        Some(unsafe { &*(inner as *const [MaybeUninit<T>] as *const [T]) })
    }
}

impl<T: 'static> Resolve for List<T> {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self> {
        let size = reader.read(addr + Self::SIZE)?;
        let items = reader.read(addr + Self::ITEMS)?;
        let items = Array::resolve(reader, items)?;
        Some(Self { addr, items, size })
    }
}

pub struct Map<K, V> {
    addr: u64,
    entries: Array<Entry<K, V>>,
    size: u32,
}

impl<K, V> Copy for Map<K, V> {}

impl<K, V> Clone for Map<K, V> {
    fn clone(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            entries: self.entries.clone(),
            size: self.size.clone(),
        }
    }
}

impl<K, V> fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Map")
            .field("addr", &self.addr)
            .field("entries", &self.entries)
            .field("size", &self.size)
            .finish()
    }
}

impl<K, V> Map<K, V> {
    const ENTRIES: u64 = 0x18;
    const SIZE: u64 = 0x20;

    pub fn size(&self) -> u32 {
        self.size
    }
}

impl<K: AnyBitPattern + 'static, V: AnyBitPattern + 'static> Map<K, V> {
    pub fn iter(self, reader: impl MemReader) -> impl Iterator<Item = (K, V)> {
        self.entries
            .iter(reader)
            .filter(|o| o._hash != 0 || o._next != 0)
            .take(self.size as _)
            .map(|o| (o.key, o.value))
    }
}

impl<K: 'static, V: 'static> Resolve for Map<K, V> {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self> {
        let size = reader.read(addr + Self::SIZE)?;
        let entries = reader.read(addr + Self::ENTRIES)?;
        let entries = Array::resolve(reader, entries)?; // reader.resolve(entries)?;
        Some(Self {
            addr,
            entries,
            size,
        })
    }
}

pub struct Set<T> {
    map: Map<T, ()>,
}

impl<T> Copy for Set<T> {}

impl<T> Clone for Set<T> {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

impl<T> fmt::Debug for Set<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Set").field("map", &self.map).finish()
    }
}

impl<T> Set<T> {
    pub fn size(&self) -> u32 {
        self.map.size
    }
}

impl<T: AnyBitPattern + 'static> Set<T> {
    pub fn iter(self, reader: impl MemReader) -> impl Iterator<Item = T> {
        self.map.iter(reader).map(|o| o.0)
    }
}

impl<T: 'static> Resolve for Set<T> {
    fn resolve(reader: impl MemReader, addr: u64) -> Option<Self> {
        let map = Map::resolve(reader, addr)?; // reader.resolve(addr)?;
        Some(Self { map })
    }
}

#[derive(Copy, Clone, Debug, AnyBitPattern)]
#[repr(C)]
pub struct Entry<K, V> {
    _hash: u32,
    _next: u32,
    pub key: K,
    pub value: V,
}
