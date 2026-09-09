use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{
            DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            eCommunications, eConsole, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize, STGM_READ,
        },
    },
    core::{GUID, HRESULT, Interface, PCWSTR},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutputDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub(crate) struct OutputSnapshot {
    pub console: String,
    pub multimedia: String,
    pub communications: String,
}

struct ComApartment(bool);

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        match unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
            Ok(()) => Ok(Self(true)),
            Err(error) if error.code() == windows::Win32::Foundation::RPC_E_CHANGED_MODE => {
                Ok(Self(false))
            }
            Err(error) => Err(format!("Audio devices: {error}")),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
    let id = unsafe { device.GetId()? };
    let value = unsafe { id.to_string()? };
    unsafe { CoTaskMemFree(Some(id.as_ptr().cast())) };
    Ok(value)
}

fn friendly_name(device: &IMMDevice) -> windows::core::Result<String> {
    let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
    Ok(unsafe { store.GetValue(&PKEY_Device_FriendlyName)? }.to_string())
}

pub(crate) fn output_devices() -> Result<Vec<OutputDevice>, String> {
    let _apartment = ComApartment::initialize()?;
    let enumerator = enumerator().map_err(|error| format!("Audio devices: {error}"))?;
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        .map_err(|error| format!("Audio devices: {error}"))?;
    let count =
        unsafe { collection.GetCount() }.map_err(|error| format!("Audio devices: {error}"))?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device =
            unsafe { collection.Item(index) }.map_err(|error| format!("Audio devices: {error}"))?;
        devices.push(OutputDevice {
            id: device_id(&device).map_err(|error| format!("Audio devices: {error}"))?,
            name: friendly_name(&device).map_err(|error| format!("Audio devices: {error}"))?,
        });
    }
    devices.sort_by_key(|device| device.name.to_lowercase());
    Ok(devices)
}

fn default_id(
    enumerator: &IMMDeviceEnumerator,
    role: windows::Win32::Media::Audio::ERole,
) -> windows::core::Result<String> {
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, role)? };
    device_id(&device)
}

pub(crate) fn default_output_snapshot() -> Result<OutputSnapshot, String> {
    let _apartment = ComApartment::initialize()?;
    let enumerator = enumerator().map_err(|error| format!("Audio output: {error}"))?;
    Ok(OutputSnapshot {
        console: default_id(&enumerator, eConsole)
            .map_err(|error| format!("Audio output: {error}"))?,
        multimedia: default_id(&enumerator, eMultimedia)
            .map_err(|error| format!("Audio output: {error}"))?,
        communications: default_id(&enumerator, eCommunications)
            .map_err(|error| format!("Audio output: {error}"))?,
    })
}

windows::core::imp::define_interface!(
    IPolicyConfigVista,
    IPolicyConfigVista_Vtbl,
    0x568b9108_44bf_40b4_9006_86afe5b5a620
);
windows::core::imp::interface_hierarchy!(IPolicyConfigVista, windows::core::IUnknown);

#[repr(C)]
pub struct IPolicyConfigVista_Vtbl {
    base__: windows::core::IUnknown_Vtbl,
    unused: [usize; 10],
    set_default_endpoint: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        PCWSTR,
        windows::Win32::Media::Audio::ERole,
    ) -> HRESULT,
    set_endpoint_visibility: usize,
}

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x294935ce_f637_4e7c_a41b_ab255460b862);

fn set_default_endpoint(
    id: &str,
    role: windows::Win32::Media::Audio::ERole,
) -> windows::core::Result<()> {
    let policy: IPolicyConfigVista =
        unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)? };
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    unsafe {
        (Interface::vtable(&policy).set_default_endpoint)(
            Interface::as_raw(&policy),
            PCWSTR(wide.as_ptr()),
            role,
        )
        .ok()
    }
}

fn set_roles(console: &str, multimedia: &str, communications: &str) -> Result<(), String> {
    let _apartment = ComApartment::initialize()?;
    set_default_endpoint(console, eConsole).map_err(|error| format!("Audio output: {error}"))?;
    set_default_endpoint(multimedia, eMultimedia)
        .map_err(|error| format!("Audio output: {error}"))?;
    set_default_endpoint(communications, eCommunications)
        .map_err(|error| format!("Audio output: {error}"))
}

pub(crate) fn select_output(id: &str) -> Result<(), String> {
    set_roles(id, id, id)
}

pub(crate) fn restore_output(snapshot: &OutputSnapshot) -> Result<(), String> {
    set_roles(
        &snapshot.console,
        &snapshot.multimedia,
        &snapshot.communications,
    )
}
