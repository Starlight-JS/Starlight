/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
use super::{Context};
use crate::gc::cell::{GcCell, GcPointer, Trace};
use crate::gc::snapshot::deserializer::Deserializable;
use dashmap::DashMap;
use std::sync::atomic::Ordering;
use std::{mem::MaybeUninit, sync::atomic::AtomicU32};
pub struct SymbolTable {
    pub(crate) symbols: DashMap<&'static str, u32>,
    pub(crate) ids: DashMap<u32, &'static str>,
    key: AtomicU32,
}
impl Drop for SymbolTable {
    fn drop(&mut self) {
        for entry in self.ids.iter_mut() {
            let key = entry.value();
            unsafe {
                let _ = Box::from_raw((*key) as *const _ as *mut str);
            }
        }
        self.symbols.clear();
        self.ids.clear();
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
impl SymbolTable {
    pub fn new() -> Self {
        Self {
            symbols: DashMap::with_capacity(0),
            ids: DashMap::with_capacity(0),
            key: AtomicU32::new(128),
        }
    }

    pub fn description(&self, symbol: SymbolID) -> &'static str {
        *self.ids.get(&symbol.0).unwrap()
    }
    pub fn intern(&self, val: impl AsRef<str>) -> SymbolID {
        let string = val.as_ref();
        if let Some(key) = self.symbols.get(string) {
            return SymbolID(*key.value());
        }

        let string = Box::leak(string.to_string().into_boxed_str());
        let make_new_key = || self.key.fetch_add(1, Ordering::Relaxed);
        let key = *self
            .symbols
            .entry(string)
            .or_insert_with(make_new_key)
            .value();
        self.ids.insert(key, string);
        SymbolID(key)
    }
}

macro_rules! builtin_symbols {
    ($m: ident) => {
        $m! {
            /*PROTOTYPE prototype 0,
            TO_STRING toString 1,
            CONSTRUCTOR constructor 2,
            LENGTH length 3,
            BYTE_LENGTH byteLength 4,
            GET get 5,
            SET set 6,
            CALL call 7,
            APPLY apply 8*/

        }
    };
}

macro_rules! def_sid {
    ($($id: ident $val: ident $ix: expr),*) => {
        $(pub const $id: SymbolID = SymbolID($ix);)*
    };
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct SymbolID(pub(crate) u32);

impl SymbolID {
    builtin_symbols! {
        def_sid
    }

    pub const PUBLIC_START: SymbolID = Self(128);
}
/// Runtime symbol type.
///
///
/// This type is used as property names and inside JsSymbol.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum Symbol {
    /// Interned string.
    Key(SymbolID),
    /// Private symbol. You can't create it in JS world.
    Private(SymbolID),
    /// Represents index value, this variant is used when you can definetely put array
    /// index inside u32 so it does not take space in interner gc.
    Index(u32),
}

macro_rules! def_sym {
    ($($id: ident $val: ident $ix: expr),*) => {
        $(
            pub const $id: Symbol = Symbol::Key(SymbolID::$id);
        )*
    };
}

impl Symbol {
    builtin_symbols! {
        def_sym
    }
    pub fn private(self) -> Self {
        match self {
            Self::Key(x) => Self::Private(x),
            _ => unreachable!(),
        }
    }
    pub fn is_index(self) -> bool {
        /*match self {
            Self::Index(_) => true,
            _ => false,
        }*/
        matches!(self, Self::Index(_))
    }
    pub fn get_index(self) -> u32 {
        match self {
            Self::Index(x) => x,
            _ => unreachable!(),
        }
    }
    pub fn is_key(self) -> bool {
        !self.is_index()
    }
}
impl GcCell for Symbol {
    fn deser_pair(&self) -> (usize, usize) {
        (Self::deserialize as _, Self::allocate as _)
    }
}
unsafe impl Trace for Symbol {}

pub const DUMMY_SYMBOL: Symbol = Symbol::Key(SymbolID(0));

#[no_mangle]
#[doc(hidden)]
pub static mut SYMBOL_TABLE: MaybeUninit<SymbolTable> = MaybeUninit::uninit();

macro_rules! globals {
    ($($id: ident $val: ident $ix: expr),*) => {
       $( pub static $id: &'static str = stringify!($val);)*
    };
}
builtin_symbols!(globals);
macro_rules! intern_builtins {
    ($($id: ident $val: ident $ix: expr),*) => {
        let mut _symtab = symbol_table();
        $(
            _symtab.ids.insert($ix,$id);
            _symtab.symbols.insert($id,$ix);
        )*
    };
}
pub(crate) fn initialize_symbol_table() {
    unsafe {
        SYMBOL_TABLE.as_mut_ptr().write(SymbolTable::new());
        LENGTH = "length".intern();
    }
    builtin_symbols!(intern_builtins);
}

pub fn length_id() -> Symbol {
    unsafe { LENGTH }
}
pub fn symbol_table() -> &'static SymbolTable {
    unsafe { &*SYMBOL_TABLE.as_ptr() }
}
pub trait Internable {
    fn intern(&self) -> Symbol;
}

impl Internable for str {
    fn intern(&self) -> Symbol {
        Symbol::Key(symbol_table().intern(self))
    }
}

impl Internable for String {
    fn intern(&self) -> Symbol {
        Symbol::Key(symbol_table().intern(self))
    }
}

impl Internable for u32 {
    fn intern(&self) -> Symbol {
        Symbol::Index(*self)
    }
}

impl Internable for usize {
    fn intern(&self) -> Symbol {
        if *self as u32 as usize == *self {
            return (*self as u32).intern();
        }
        self.to_string().intern()
    }
}

pub struct JsSymbol {
    pub(crate) sym: Symbol,
}

impl JsSymbol {
    pub fn new(ctx: &mut Context, sym: Symbol) -> GcPointer<Self> {
        ctx.heap().allocate(Self { sym })
    }

    pub fn symbol(&self) -> Symbol {
        self.sym
    }
}

unsafe impl Trace for JsSymbol {}
impl GcCell for JsSymbol {
    fn deser_pair(&self) -> (usize, usize) {
        (Self::deserialize as _, Self::allocate as _)
    }
}

impl std::fmt::Display for SymbolID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", symbol_table().description(*self))
    }
}

static mut LENGTH: Symbol = Symbol::Key(SymbolID(0));
