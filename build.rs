//! Préparation avant la compilation : sous Windows, on range l'icône du
//! lecteur (et son nom) dans le .exe lui-même, pour que l'explorateur, le
//! bureau et le menu Démarrer l'affichent.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icone/icone.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut ressources = winresource::WindowsResource::new();
        ressources
            .set_icon("assets/icone/icone.ico")
            .set("ProductName", "Doll//Wave")
            .set("FileDescription", "Doll//Wave");
        ressources
            .compile()
            .expect("impossible de ranger l'icône dans le .exe");
    }
}
