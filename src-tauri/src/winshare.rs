//! El archivo de video hacia otras apps de Windows: al portapapeles (para pegarlo con
//! Ctrl+V en WhatsApp, Discord, Telegram…) o por la ventana "Compartir" del sistema.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use windows::ApplicationModel::DataTransfer::{Clipboard, DataPackage, DataPackageOperation, DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::TypedEventHandler;
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
use windows::core::{HSTRING, Interface, Ref, factory};
use windows_collections::IIterable;

/// Archivo listo para entregar a Windows.
pub struct SharedFile(StorageFile);

// SAFETY: `StorageFile` es un objeto ágil de WinRT: se puede usar desde cualquier hilo.
unsafe impl Send for SharedFile {}

impl SharedFile {
    /// Espera a Windows: no llamar desde el hilo de la interfaz.
    pub fn open(path: &Path) -> Result<Self> {
        // WinRT solo acepta rutas absolutas con barras invertidas.
        let path = path.display().to_string().replace('/', "\\");
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_str())).and_then(|op| op.join()).with_context(|| format!("no se pudo abrir {path}"))?;
        Ok(Self(file))
    }

    fn items(&self) -> windows::core::Result<IIterable<IStorageItem>> {
        Ok(IIterable::from(vec![Some(self.0.cast::<IStorageItem>()?)]))
    }
}

/// Copia el archivo al portapapeles, como "Copiar" en el Explorador. Va en el hilo de la interfaz.
pub fn copy_to_clipboard(file: &SharedFile) -> Result<()> {
    let data = DataPackage::new()?;
    data.SetRequestedOperation(DataPackageOperation::Copy)?;
    data.SetStorageItems(&file.items()?, true)?;
    Clipboard::SetContent(&data)?;
    // Para que siga en el portapapeles aunque se cierre la app.
    Clipboard::Flush()?;
    Ok(())
}

/// Lo que entrega la ventana "Compartir" cuando Windows lo pide.
static PENDING: Mutex<Option<(SharedFile, String)>> = Mutex::new(None);
/// Ventanas que ya tienen registrado el manejador de `DataRequested`.
static REGISTERED: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Abre la ventana "Compartir" de Windows con el archivo. Va en el hilo de la interfaz.
pub fn show_share_ui(hwnd: HWND, file: SharedFile, title: &str) -> Result<()> {
    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    // SAFETY: `hwnd` es la ventana principal, viva mientras corre la app.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }?;
    *PENDING.lock().unwrap() = Some((file, title.to_string()));
    let mut registered = REGISTERED.lock().unwrap();
    if !registered.contains(&(hwnd.0 as isize)) {
        manager.DataRequested(&TypedEventHandler::new(|_, args: Ref<DataRequestedEventArgs>| {
            if let (Some(args), Some((file, title))) = (args.as_ref(), PENDING.lock().unwrap().as_ref()) {
                let data = args.Request()?.Data()?;
                data.Properties()?.SetTitle(&HSTRING::from(title.as_str()))?;
                data.SetStorageItems(&file.items()?, true)?;
            }
            Ok(())
        }))?;
        registered.push(hwnd.0 as isize);
    }
    // SAFETY: igual que arriba.
    unsafe { interop.ShowShareUIForWindow(hwnd) }?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pone un archivo en el portapapeles (lo reemplaza): `cargo test --lib winshare -- --ignored`
    #[test]
    #[ignore]
    fn copies_a_file_to_the_clipboard() {
        let path = std::env::temp_dir().join("ggswp-portapapeles.txt");
        std::fs::write(&path, "ggswp").unwrap();
        let file = std::thread::spawn({
            let path = path.clone();
            move || SharedFile::open(&path).unwrap()
        })
        .join()
        .unwrap();
        // Como el hilo de la interfaz: apartamento de un solo hilo.
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) }.unwrap();
        copy_to_clipboard(&file).unwrap();
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", "(Get-Clipboard -Format FileDropList).FullName"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), path.display().to_string());
    }
}
