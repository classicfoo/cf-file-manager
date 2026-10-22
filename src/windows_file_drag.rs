#![allow(non_snake_case)]

use std::{
    collections::HashMap,
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{copy_nonoverlapping, null, null_mut},
    sync::Mutex,
    sync::atomic::{AtomicU32, Ordering},
};

use windows_sys::{
    Win32::{
        Foundation::{
            DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
            DV_E_FORMATETC, DV_E_TYMED, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER,
            GlobalFree, OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK,
        },
        System::{
            Com::{
                DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IDataObject, IEnumFORMATETC, STGMEDIUM,
                STGMEDIUM_0, TYMED_HGLOBAL,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{
                GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
            },
            Ole::{
                CF_HDROP, DROPEFFECT_COPY, DoDragDrop, OleInitialize, OleUninitialize,
                ReleaseStgMedium,
            },
        },
        UI::Shell::{
            CFSTR_LOGICALPERFORMEDDROPEFFECT, CFSTR_PERFORMEDDROPEFFECT, CFSTR_PREFERREDDROPEFFECT,
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
    preferred_drop_effect_format: u16,
    outcome_formats: [u16; 2],
    outcome_data: Mutex<HashMap<u16, Vec<u8>>>,
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
    formats: Vec<FORMATETC>,
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
    let preferred_drop_effect_format = register_clipboard_format(CFSTR_PREFERREDDROPEFFECT)?;
    let outcome_formats = [
        register_clipboard_format(CFSTR_PERFORMEDDROPEFFECT)?,
        register_clipboard_format(CFSTR_LOGICALPERFORMEDDROPEFFECT)?,
    ];
    let mut wide_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide_path.push(0);
    wide_path.push(0);

    let data_object = Box::into_raw(Box::new(DataObject {
        vtable: &DATA_OBJECT_VTABLE,
        references: AtomicU32::new(1),
        paths: wide_path,
        preferred_drop_effect_format,
        outcome_formats,
        outcome_data: Mutex::new(HashMap::new()),
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
        DRAGDROP_S_DROP if effect & DROPEFFECT_COPY != 0 => Ok(true),
        DRAGDROP_S_DROP | DRAGDROP_S_CANCEL => Ok(false),
        error if error < 0 => Err(format!(
            "Windows drag operation failed (0x{:08X})",
            error as u32
        )),
        _ => Ok(false),
    }
}

fn register_clipboard_format(name: windows_sys::core::PCWSTR) -> Result<u16, String> {
    let format = unsafe { RegisterClipboardFormatW(name) };
    if format == 0 {
        Err("Windows could not register a drag-and-drop format.".to_owned())
    } else {
        Ok(format as u16)
    }
}

fn supported_format(format: *const FORMATETC, object: &DataObject) -> HRESULT {
    if format.is_null() {
        return E_POINTER;
    }
    let format = unsafe { &*format };
    if format.dwAspect != DVASPECT_CONTENT || format.lindex != -1 {
        return DV_E_FORMATETC;
    }
    if format.tymed & TYMED_HGLOBAL as u32 == 0 {
        return DV_E_TYMED;
    }
    if format.cfFormat == CF_HDROP || format.cfFormat == object.preferred_drop_effect_format {
        return S_OK;
    }
    if object
        .outcome_data
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&format.cfFormat)
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
    if this.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this as *mut DataObject) };
    let validation = supported_format(format, object);
    if validation != S_OK {
        return validation;
    }

    let format = unsafe { &*format };
    let bytes = if format.cfFormat == CF_HDROP {
        let header_size = size_of::<windows_sys::Win32::UI::Shell::DROPFILES>();
        let mut bytes = vec![0u8; header_size + object.paths.len() * size_of::<u16>()];
        bytes[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
        for (index, character) in object.paths.iter().copied().enumerate() {
            let offset = header_size + index * size_of::<u16>();
            bytes[offset..offset + 2].copy_from_slice(&character.to_le_bytes());
        }
        bytes
    } else if format.cfFormat == object.preferred_drop_effect_format {
        DROPEFFECT_COPY.to_le_bytes().to_vec()
    } else {
        let outcome_data = object
            .outcome_data
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = outcome_data.get(&format.cfFormat) else {
            return DV_E_FORMATETC;
        };
        bytes.clone()
    };

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
    this: *mut IDataObject,
    format: *const FORMATETC,
) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this as *mut DataObject) };
    supported_format(format, object)
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
    this: *mut IDataObject,
    format: *const FORMATETC,
    medium: *const STGMEDIUM,
    release: i32,
) -> HRESULT {
    if this.is_null() || format.is_null() || medium.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this as *mut DataObject) };
    let format = unsafe { &*format };
    let medium = unsafe { &*medium };
    if format.cfFormat == CF_HDROP
        || format.cfFormat == object.preferred_drop_effect_format
        || format.dwAspect != DVASPECT_CONTENT
        || format.lindex != -1
    {
        return E_NOTIMPL;
    }
    if format.tymed & TYMED_HGLOBAL as u32 == 0 || medium.tymed != TYMED_HGLOBAL as u32 {
        return DV_E_TYMED;
    }

    let handle = unsafe { medium.u.hGlobal };
    let length = unsafe { GlobalSize(handle) };
    let locked = unsafe { GlobalLock(handle) };
    if locked.is_null() {
        return E_OUTOFMEMORY;
    }
    let bytes = unsafe { std::slice::from_raw_parts(locked.cast::<u8>(), length) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    object
        .outcome_data
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(format.cfFormat, bytes);
    if release != 0 {
        unsafe { ReleaseStgMedium(medium as *const STGMEDIUM as *mut STGMEDIUM) };
    }
    S_OK
}

unsafe extern "system" fn data_enum_format(
    this: *mut IDataObject,
    direction: u32,
    output: *mut *mut IEnumFORMATETC,
) -> HRESULT {
    if output.is_null() || this.is_null() {
        return E_POINTER;
    }
    unsafe { *output = null_mut() };
    let object = unsafe { &*(this as *mut DataObject) };
    let formats = match direction {
        direction if direction == DATADIR_GET as u32 => {
            let mut formats = vec![
                file_drop_format(),
                global_memory_format(object.preferred_drop_effect_format),
            ];
            let outcome_data = object
                .outcome_data
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            formats.extend(
                outcome_data
                    .keys()
                    .map(|format| global_memory_format(*format)),
            );
            formats
        }
        direction if direction == windows_sys::Win32::System::Com::DATADIR_SET as u32 => object
            .outcome_formats
            .iter()
            .copied()
            .map(global_memory_format)
            .collect(),
        _ => return E_NOTIMPL,
    };
    let enumerator = Box::new(FormatEnumerator {
        vtable: &ENUM_FORMATETC_VTABLE,
        references: AtomicU32::new(1),
        formats,
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
    let mut copied = 0;
    while copied < count && object.index < object.formats.len() as u32 {
        unsafe { *output.add(copied as usize) = object.formats[object.index as usize] };
        object.index += 1;
        copied += 1;
    }
    if !fetched.is_null() {
        unsafe { *fetched = copied };
    }
    if copied == count { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_skip(this: *mut c_void, count: u32) -> HRESULT {
    let object = unsafe { &mut *(this as *mut FormatEnumerator) };
    let available = (object.formats.len() as u32).saturating_sub(object.index);
    let skipped = count.min(available);
    object.index += skipped;
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
        formats: object.formats.clone(),
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

fn global_memory_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
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
