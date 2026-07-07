// Empêche l'ouverture d'une console supplémentaire en release sur Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lavoir_lib::run()
}
