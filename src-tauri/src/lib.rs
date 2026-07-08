mod doctor;
mod janitor;
mod laver;
mod recuperer;
mod settings;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(laver::ExifState::default())
        .manage(recuperer::DownloadManager::default())
        .manage(recuperer::LaunchUpdate::default())
        .setup(|app| {
            let handle = app.handle().clone();
            // Purge les résidus d'une session précédente, puis lance le balayage
            // périodique. Ne tourne que dans l'instance primaire (cf. janitor).
            janitor::purge_all();
            janitor::spawn_periodic(handle.clone());
            recuperer::spawn_launch_check(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            doctor::sidecar_versions,
            laver::inspect_files,
            laver::clean_files,
            laver::extract_thumbnail,
            recuperer::probe_url,
            recuperer::start_download,
            recuperer::cancel_download,
            recuperer::default_destination,
            recuperer::update_ytdlp,
            recuperer::launch_update_status,
            settings::get_settings,
            settings::set_settings,
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement de lavoir")
        .run(|handle, event| {
            // À la fermeture : couper les téléchargements en cours (sinon leurs
            // process survivraient) puis effacer le temp. Rien ne reste.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                handle.state::<recuperer::DownloadManager>().cancel_all();
                janitor::purge_all();
            }
        });
}
