mod doctor;

use tauri::Manager;

pub fn run() {
    // `lavoir.exe --doctor` : vérifie la résolution des moteurs embarqués sans
    // ouvrir de fenêtre. En release (subsystem windows), la sortie n'est visible
    // que redirigée : `.\lavoir.exe --doctor > doctor.txt`.
    if std::env::args().any(|a| a == "--doctor") {
        std::process::exit(doctor::run_cli());
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![doctor::sidecar_versions])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de lavoir");
}
