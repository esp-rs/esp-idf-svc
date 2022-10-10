#[cfg(all(
    esp_idf_esp_tls_server_cert_select_hook,
    esp_idf_comp_esp_http_server_enabled
))]
pub mod cert_select;
#[cfg(esp_idf_comp_esp_http_client_enabled)]
pub mod client;
#[cfg(esp_idf_comp_esp_http_server_enabled)]
pub mod server;
