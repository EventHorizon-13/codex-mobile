fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/codex.ico");
        resource.set("ProductName", "Codex Chat Mobile");
        resource.set("FileDescription", "Codex Chat Mobile");
        resource.set("CompanyName", "Codex Mobile");
        resource.compile().expect("compile Windows application resources");
    }
}

