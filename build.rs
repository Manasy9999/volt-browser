// Embarque l'icône de Volty dans volt.exe (Explorateur, barre des tâches).
fn main() {
    println!("cargo:rerun-if-changed=logos/volt.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new().set_icon("logos/volt.ico").compile().expect("icône");
    }
}
