mod backend;
mod flashing;
mod model;
mod protocol;
mod settings;
mod storage;
mod transport;
// GTK's editable ComboBoxText is still needed for user-entered serial paths and BLE IDs.
#[allow(deprecated)]
mod ui;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    gtk::gio::resources_register_include!("meshtastic.gresource")
        .expect("Bundled application logos");
    if std::env::args().any(|arg| arg == "--probe-usb") {
        return backend::probe_usb();
    }
    let app = gtk::Application::builder()
        .application_id("io.github.meshtasticgtk.Client")
        .build();
    app.connect_activate(ui::build);
    app.run()
}
