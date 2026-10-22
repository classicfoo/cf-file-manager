#![allow(non_snake_case)]

use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{copy_nonoverlapping, null, null_mut},
    sync::atomic::{AtomicU32, Ordering},
};

use windows_sys::{
    Win32::{
        Foundation::{
            DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
            DV_E_FORMATETC, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, GlobalFree,
            OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK,
        },
        System::{
            Com::{
                DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IDataObject, IEnumFORMATETC, STGMEDIUM,
                STGMEDIUM_0, TYMED_HGLOBAL,
            },
            Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_HDROP, DROPEFFECT_COPY, DoDragDrop, OleInitialize, OleUninitialize},
        },
    },
    core::{GUID, HRESULT},
};

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_IENUMFORMATETC: GUID = GUID::from_u128(0x00000103_0000_0000_c000_000000000046);
const IID_IDROPSOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);
const MK_LBUTTON: u32 = 0x0001;

#[repr(C)]
struct DataObjectVTable {
    QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetData:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    GetDataHere:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    QueryGetData: unsafe extern "system" fn(*mut IDataObject, *const FORMATETC) -> HRESULT,
    GetCanonicalFormatEtc:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    SetData: unsafe extern "system" fn(
        *mut IDataObject,
        *const FORMATETC,
        *const STGMEDIUM,
        i32,
    ) -> HRESULT,
    EnumFormatEtc:
        unsafe extern "system" fn(*mut IDataObject, u32, *mut *mut IEnumFORMATETC) -> HRESULT,
    DAdvise: unsafe extern "system" fn(
        *mut IDataObject,
        *const FORMATETC,
        u32,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    DUnadvise: unsafe extern "system" fn(*mut IDataObject, u32) -> HRESULT,
    EnumDAdvise: unsafe extern "system" fn(*mut IDataObject, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DataObject {
    vtable: *const DataObjectVTable,
    references: AtomicU32,
    paths: Vec<u16>,
}

#[repr(C)]
struct DropSourceVTable {
    QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    QueryContinueDrag: unsafe extern "system" fn(*mut c_void, i32, u32) -> HRESULT,
    GiveFeedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

#[repr(C)]
struct DropSource {
    vtable: *const DropSourceVTable,
    references: AtomicU32,
}

#[repr(C)]
struct EnumFormatEtcVTable {
    QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    Next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> HRESULT,
    Skip: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    Reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    Clone: unsafe extern "system" fn(*mut c_void, *mut *mut IEnumFORMATETC) -> HRESULT,
}

#[repr(C)]
struct FormatEnumerator {
    vtable: *const EnumFormatEtcVTable,
    references: AtomicU32,
    index: u32,
}

static DATA_OBJECT_VTABLE: DataObjectVTable = DataObjectVTable {
    QueryInterface: data_query_interface,
    AddRef: data_add_ref,
    Release: data_release,
    GetData: data_get,
    GetDataHere: data_get_here,
    QueryGetData: data_query_get,
    GetCanonicalFormatEtc: data_get_canonical,
    SetData: data_set,
    EnumFormatEtc: data_enum_format,
    DAdvise: data_advise,
    DUnadvise: data_unadvise,
    EnumDAdvise: data_enum_advise,
};

static DROP_SOURCE_VTABLE: DropSourceVTable = DropSourceVTable {
    QueryInterface: drop_source_query_interface,
    AddRef: drop_source_add_ref,
    Release: drop_source_release,
    QueryContinueDrag: drop_source_query_continue,
    GiveFeedback: drop_source_give_feedback,
};

static ENUM_FORMATETC_VTABLE: EnumFormatEtcVTable = EnumFormatEtcVTable {
    QueryInterface: enum_query_interface,
    AddRef: enum_add_ref,
    Release: enum_release,
    Next: enum_next,
    Skip: enum_skip,
    Reset: enum_reset,
    Clone: enum_clone,
};

pub(super) fn drag_path(path: &Path) -> Result<bool, String> {
    let mut wide_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide_path.push(0);
    wide_path.push(0);

    let data_object = Box::into_raw(Box::new(DataObject {
        vtable: &DATA_OBJECT_VTABLE,
        references: AtomicU32::new(1),
        paths: wide_path,
    })) as *mut c_void;
    let drop_source = Box::into_raw(Box::new(DropSource {
        vtable: &DROP_SOURCE_VTABLE,
        references: AtomicU32::new(1),
    })) as *mut c_void;

    let initialize_result = unsafe { OleInitialize(null()) };
    if initialize_result < 0 {
        unsafe {
            data_release(data_object);
            drop_source_release(drop_source);
        }
        return Err(format!(
            "OLE initialization failed (0x{:08X})",
            initialize_result as u32
        ));
    }

    let mut effect = 0;
    let result = unsafe {
        DoDragDrop(
            data_object as IDataObject,
            drop_source,
            DROPEFFECT_COPY,
            &mut effect,
        )
    };
    unsafe {
        data_release(data_object);
        drop_source_release(drop_source);
        OleUninitialize();
    }

    match result {
        DRAGDROP_S_DROP => Ok(effect & DROPEFFECT_COPY != 0),
        DRAGDROP_S_CANCEL => Ok(false),
        error if error < 0 => Err(format!(
            "Windows drag operation failed (0x{:08X})",
            error as u32
        )),
        _ => Ok(effect & DROPEFFECT_COPY != 0),
    }
}

fn supported_format(format: *const FORMATETC) -> HRESULT {
    if format.is_null() {
        return E_POINTER;
    }
    let format = unsafe { &*format };
    if format.cfFormat == CF_HDROP
        && format.dwAspect == DVASPECT_CONTENT
        && format.lindex == -1
        && format.tymed & TYMED_HGLOBAL as u32 != 0
    {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn data_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        query_interface(
            this,
            iid,
            output,
            &[IID_IUNKNOWN, IID_IDATAOBJECT],
            data_add_ref,
        )
    }
}

unsafe extern "system" fn data_add_ref(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut DataObject) };
    object.references.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn data_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut DataObject) };
    let remaining = object.references.fetch_sub(1, Ordering::Release) - 1;
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        unsafe { drop(Box::from_raw(this as *mut DataObject)) };
    }
    remaining
}

unsafe extern "system" fn data_get(
    this: *mut IDataObject,
    format: *const FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    if medium.is_null() {
        return E_POINTER;
    }
    let validation = supported_format(format);
    if validation != S_OK {
        return validation;
    }

    let object = unsafe { &*(this as *mut DataObject) };
    let header_size = size_of::<windows_sys::Win32::UI::Shell::DROPFILES>();
    let mut bytes = vec![0u8; header_size + object.paths.len() * size_of::<u16>()];
    bytes[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
    for (index, character) in object.paths.iter().copied().enumerate() {
        let offset = header_size + index * size_of::<u16>();
        bytes[offset..offset + 2].copy_from_slice(&character.to_le_bytes());
    }

    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()) };
    if global.is_null() {
        return E_OUTOFMEMORY;
    }
    let locked = unsafe { GlobalLock(global) };
    if locked.is_null() {
        unsafe { GlobalFree(global) };
        return E_OUTOFMEMORY;
    }
    unsafe {
        copy_nonoverlapping(bytes.as_ptr(), locked.cast::<u8>(), bytes.len());
        GlobalUnlock(global);
        *medium = zeroed();
        (*medium).tymed = TYMED_HGLOBAL as u32;
        (*medium).u = STGMEDIUM_0 { hGlobal: global };
    }
    S_OK
}

unsafe extern "system" fn data_get_here(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _medium: *mut STGMEDIUM,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_query_get(
    _this: *mut IDataObject,
    format: *const FORMATETC,
) -> HRESULT {
    supported_format(format)
}

unsafe extern "system" fn data_get_canonical(
    _this: *mut IDataObject,
    input: *const FORMATETC,
    output: *mut FORMATETC,
) -> HRESULT {
    if input.is_null() || output.is_null() {
        return E_POINTER;
    }
    unsafe {
        *output = *input;
        (*output).ptd = null_mut();
    }
    DATA_S_SAMEFORMATETC
}

unsafe extern "system" fn data_set(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _medium: *const STGMEDIUM,
    _release: i32,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_enum_format(
    _this: *mut IDataObject,
    direction: u32,
    output: *mut *mut IEnumFORMATETC,
) -> HRESULT {
    if output.is_null() {
        return E_POINTER;
    }
    unsafe { *output = null_mut() };
    if direction != DATADIR_GET as u32 {
        return E_NOTIMPL;
    }
    let enumerator = Box::new(FormatEnumerator {
        vtable: &ENUM_FORMATETC_VTABLE,
        references: AtomicU32::new(1),
        index: 0,
    });
    unsafe { *output = Box::into_raw(enumerator) as *mut IEnumFORMATETC };
    S_OK
}

unsafe extern "system" fn data_advise(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _flags: u32,
    _sink: *mut c_void,
    _connection: *mut u32,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_unadvise(_this: *mut IDataObject, _connection: u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_enum_advise(
    _this: *mut IDataObject,
    _output: *mut *mut c_void,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn drop_source_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        query_interface(
            this,
            iid,
            output,
            &[IID_IUNKNOWN, IID_IDROPSOURCE],
            drop_source_add_ref,
        )
    }
}

unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut DropSource) };
    object.references.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut DropSource) };
    let remaining = object.references.fetch_sub(1, Ordering::Release) - 1;
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        unsafe { drop(Box::from_raw(this as *mut DropSource)) };
    }
    remaining
}

unsafe extern "system" fn drop_source_query_continue(
    _this: *mut c_void,
    escape_pressed: i32,
    key_state: u32,
) -> HRESULT {
    if escape_pressed != 0 {
        DRAGDROP_S_CANCEL
    } else if key_state & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

unsafe extern "system" fn enum_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        query_interface(
            this,
            iid,
            output,
            &[IID_IUNKNOWN, IID_IENUMFORMATETC],
            enum_add_ref,
        )
    }
}

unsafe extern "system" fn enum_add_ref(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut FormatEnumerator) };
    object.references.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn enum_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *mut FormatEnumerator) };
    let remaining = object.references.fetch_sub(1, Ordering::Release) - 1;
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        unsafe { drop(Box::from_raw(this as *mut FormatEnumerator)) };
    }
    remaining
}

unsafe extern "system" fn enum_next(
    this: *mut c_void,
    count: u32,
    output: *mut FORMATETC,
    fetched: *mut u32,
) -> HRESULT {
    if count == 0 {
        if !fetched.is_null() {
            unsafe { *fetched = 0 };
        }
        return S_OK;
    }
    if output.is_null() || (count != 1 && fetched.is_null()) {
        return E_POINTER;
    }
    let object = unsafe { &mut *(this as *mut FormatEnumerator) };
    let available = u32::from(object.index == 0);
    let copied = count.min(available);
    if copied == 1 {
        unsafe {
            *output = file_drop_format();
            object.index = 1;
        }
    }
    if !fetched.is_null() {
        unsafe { *fetched = copied };
    }
    if copied == count { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_skip(this: *mut c_void, count: u32) -> HRESULT {
    let object = unsafe { &mut *(this as *mut FormatEnumerator) };
    let available = u32::from(object.index == 0);
    object.index = 1;
    if count <= available { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_reset(this: *mut c_void) -> HRESULT {
    unsafe { (*(this as *mut FormatEnumerator)).index = 0 };
    S_OK
}

unsafe extern "system" fn enum_clone(
    this: *mut c_void,
    output: *mut *mut IEnumFORMATETC,
) -> HRESULT {
    if output.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this as *mut FormatEnumerator) };
    let clone = Box::new(FormatEnumerator {
        vtable: &ENUM_FORMATETC_VTABLE,
        references: AtomicU32::new(1),
        index: object.index,
    });
    unsafe { *output = Box::into_raw(clone) as *mut IEnumFORMATETC };
    S_OK
}

fn file_drop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    }
}

unsafe fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
    supported: &[GUID],
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
) -> HRESULT {
    if output.is_null() || iid.is_null() {
        return E_POINTER;
    }
    unsafe { *output = null_mut() };
    let requested = unsafe { &*iid };
    if supported
        .iter()
        .any(|candidate| guid_equals(requested, candidate))
    {
        unsafe {
            *output = this;
            add_ref(this);
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

fn guid_equals(left: &GUID, right: &GUID) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}
