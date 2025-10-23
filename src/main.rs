// V1.0

#![no_std]
#![no_main]

use esp_idf_sys::*;
use esp_idf_hal::{delay::Ets, i2c::I2cDriver, peripherals::Peripherals, prelude::*};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
    ipv4::IpInfo
};
use bme280::i2c::BME280;
use log::{info, error};
use anyhow::{Result, anyhow};
use serde_json::{json, Value};
use alloc::{boxed::Box, string::{String, ToString}, ffi::CString, format, vec::Vec};
use core::ffi::c_void;
use sha2::{Digest, Sha256};
extern crate alloc;

// OTA Constants
const OTA_REQUEST_TOPIC: &str = "v1/devices/me/attributes/request/";
const OTA_RESPONSE_TOPIC: &str = "v1/devices/me/attributes/response/";
const OTA_FIRMWARE_REQUEST_TOPIC: &str = "v2/fw/request";
const OTA_FIRMWARE_RESPONSE_TOPIC: &str = "v2/fw/response";
const OTA_TELEMETRY_TOPIC: &str = "v1/devices/me/telemetry";

// OTA Shared Attributes
const FW_TITLE_ATTR: &str = "fw_title";
const FW_VERSION_ATTR: &str = "fw_version";
const FW_SIZE_ATTR: &str = "fw_size";
const FW_CHECKSUM_ATTR: &str = "fw_checksum";
const FW_CHECKSUM_ALG_ATTR: &str = "fw_checksum_algorithm";
const FW_STATE_ATTR: &str = "fw_state";

#[inline(always)]
fn ms_to_ticks(ms: u32) -> u32 {
    (ms as u64 * configTICK_RATE_HZ as u64 / 1000) as u32
}

fn adc_to_ppm(adc_raw: i32) -> f32 {
    let adc_min = 0.0;
    let adc_max = 3500.0;
    let ppm_min = 0.0;
    let ppm_max = 1200.0;
    let adc_f = adc_raw as f32;
    let ppm = (adc_max - adc_f) / (adc_max - adc_min) * (ppm_max - ppm_min) + ppm_min;
    if ppm < ppm_min {
        ppm_min
    } else if ppm > ppm_max {
        ppm_max
    } else {
        ppm
    }
}

#[derive(PartialEq)]
enum OtaState {
    Idle,
    Downloading,
    Downloaded,
    Verifying,
    Updating,
    Updated,
    Failed(String),
}

struct OtaManager {
    current_fw_title: String,
    current_fw_version: String,
    fw_title: Option<String>,
    fw_version: Option<String>,
    fw_size: Option<u32>,
    fw_checksum: Option<String>,
    fw_checksum_algorithm: Option<String>,
    ota_state: OtaState,
    request_id: u32,
    firmware_request_id: u32,
    current_chunk: u32,
    ota_handle: esp_ota_handle_t,
    ota_partition: *const esp_partition_t,
    received_size: usize,
    sha256_hasher: Sha256,
    partial_firmware_data: Vec<u8>,
    chunk_buffer: Vec<(u32, Vec<u8>)>,
    chunk_size: usize,
    last_chunk_received: u32,
    telemetry_counter: u32,
}

impl OtaManager {
    fn new() -> Self {
        unsafe {
            let otadata_partition = esp_partition_find_first(
                esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
                esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_OTA,
                core::ptr::null()
            );
            if !otadata_partition.is_null() {
                info!("Found otadata partition at {:p}", otadata_partition);
            } else {
                error!("No otadata partition found");
            }

            let mut ota_partitions_found = 0;
            for subtype in &[esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_0, esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_1] {
                let mut iterator = esp_partition_find(
                    esp_partition_type_t_ESP_PARTITION_TYPE_APP,
                    *subtype,
                    core::ptr::null()
                );
                while !iterator.is_null() {
                    let partition = esp_partition_get(iterator);
                    let label = core::ffi::CStr::from_ptr((*partition).label.as_ptr()).to_str().unwrap_or("unknown");
                    info!("Found OTA partition: {}, subtype: {:?}, address: 0x{:x}, size: 0x{:x}",
                        label, *subtype, (*partition).address, (*partition).size);
                    ota_partitions_found += 1;
                    iterator = esp_partition_next(iterator);
                }
                esp_partition_iterator_release(iterator);
            }
            if ota_partitions_found < 2 {
                error!("Insufficient OTA partitions found: {}. Need at least 2 for OTA.", ota_partitions_found);
            } else {
                info!("Found {} OTA partitions, sufficient for OTA", ota_partitions_found);
            }

            let running_partition = esp_ota_get_running_partition();
            if !running_partition.is_null() {
                let label = core::ffi::CStr::from_ptr((*running_partition).label.as_ptr()).to_str().unwrap_or("unknown");
                info!("Current running partition: {}, address: 0x{:x}, size: 0x{:x}",
                    label, (*running_partition).address, (*running_partition).size);
            } else {
                error!("No running partition detected");
            }
        }

        Self {
            current_fw_title: "Weather Station".to_string(),
            current_fw_version: "V1.0".to_string(),
            fw_title: None,
            fw_version: None,
            fw_size: None,
            fw_checksum: None,
            fw_checksum_algorithm: None,
            ota_state: OtaState::Idle,
            request_id: 0,
            firmware_request_id: 0,
            current_chunk: 0,
            ota_handle: 0,
            ota_partition: core::ptr::null(),
            received_size: 0,
            sha256_hasher: Sha256::new(),
            partial_firmware_data: Vec::new(),
            chunk_buffer: Vec::with_capacity(10),
            chunk_size: 4096,
            last_chunk_received: 0,
            telemetry_counter: 0,
        }
    }

    fn handle_shared_attributes(&mut self, attributes: &str, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        let attrs: Value = serde_json::from_str(attributes)?;
        info!("Raw attributes received: {}", attributes);

        let shared_attrs = attrs.get("shared").ok_or_else(|| anyhow!("Missing 'shared' object in attributes"))?;

        if let Some(fw_title) = shared_attrs.get(FW_TITLE_ATTR).and_then(|v| v.as_str()) {
            self.fw_title = Some(fw_title.trim().to_string());
            info!("Received fw_title: '{}'", fw_title);
        }
        if let Some(fw_version) = shared_attrs.get(FW_VERSION_ATTR).and_then(|v| v.as_str()) {
            self.fw_version = Some(fw_version.trim().to_string());
            info!("Received fw_version: '{}'", fw_version);
        }
        if let Some(fw_size) = shared_attrs.get(FW_SIZE_ATTR).and_then(|v| v.as_u64()) {
            self.fw_size = Some(fw_size as u32);
            info!("Received fw_size: {}", fw_size);
        }
        if let Some(fw_checksum) = shared_attrs.get(FW_CHECKSUM_ATTR).and_then(|v| v.as_str()) {
            self.fw_checksum = Some(fw_checksum.trim().to_string());
            info!("Received fw_checksum: '{}'", fw_checksum);
        }
        if let Some(fw_checksum_alg) = shared_attrs.get(FW_CHECKSUM_ALG_ATTR).and_then(|v| v.as_str()) {
            self.fw_checksum_algorithm = Some(fw_checksum_alg.trim().to_string());
            info!("Received fw_checksum_algorithm: '{}'", fw_checksum_alg);
        }

        let mut result = Ok(());
        if let (Some(fw_title), Some(fw_version)) = (&self.fw_title, &self.fw_version) {
            info!("Comparing fw_title: '{}' vs '{}', fw_version: '{}' vs '{}'", 
                fw_title, self.current_fw_title, fw_version, self.current_fw_version);
            if fw_title.trim() != self.current_fw_title.trim() || fw_version.trim() != self.current_fw_version.trim() {
                info!("New firmware available: {} {}, starting download", fw_title, fw_version);
                self.ota_state = OtaState::Downloading;
                self.firmware_request_id += 1;
                self.current_chunk = 0;
                self.received_size = 0;
                self.sha256_hasher = Sha256::new();
                self.chunk_buffer.clear();
                self.last_chunk_received = unsafe { xTaskGetTickCount() };
                unsafe {
                    self.ota_partition = esp_ota_get_next_update_partition(core::ptr::null());
                    if self.ota_partition.is_null() {
                        error!("esp_ota_get_next_update_partition failed. Attempting manual partition selection...");
                        let running_partition = esp_ota_get_running_partition();
                        if !running_partition.is_null() {
                            let label = core::ffi::CStr::from_ptr((*running_partition).label.as_ptr()).to_str().unwrap_or("unknown");
                            info!("Running partition: {}, address: 0x{:x}", label, (*running_partition).address);
                        } else {
                            error!("No running partition detected");
                        }

                        for subtype in &[esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_0, esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_APP_OTA_1] {
                            let mut iterator = esp_partition_find(
                                esp_partition_type_t_ESP_PARTITION_TYPE_APP,
                                *subtype,
                                core::ptr::null()
                            );
                            while !iterator.is_null() {
                                let partition = esp_partition_get(iterator);
                                let label = core::ffi::CStr::from_ptr((*partition).label.as_ptr()).to_str().unwrap_or("unknown");
                                info!("Checking partition: {}, subtype: {:?}, address: 0x{:x}", label, *subtype, (*partition).address);
                                if !running_partition.is_null() && partition != running_partition {
                                    self.ota_partition = partition;
                                    break;
                                }
                                iterator = esp_partition_next(iterator);
                            }
                            esp_partition_iterator_release(iterator);
                            if !self.ota_partition.is_null() {
                                break;
                            }
                        }
                    }

                    if self.ota_partition.is_null() {
                        error!("No valid OTA partition found for update");
                        self.ota_state = OtaState::Failed("No valid OTA partition found".to_string());
                        result = Err(anyhow!("No valid OTA partition found"));
                    } else {
                        let label = core::ffi::CStr::from_ptr((*self.ota_partition).label.as_ptr()).to_str().unwrap_or("unknown");
                        info!("Selected OTA partition: {}, address: 0x{:x}, size: 0x{:x}",
                            label, (*self.ota_partition).address, (*self.ota_partition).size);
                        
                        let res = esp_partition_erase_range(self.ota_partition, 0, (*self.ota_partition).size as usize);
                        if res != ESP_OK {
                            self.ota_state = OtaState::Failed(format!("Failed to erase OTA partition: {}", res));
                            result = Err(anyhow!("Failed to erase OTA partition: {}", res));
                        } else {
                            let res = esp_ota_begin(self.ota_partition, self.fw_size.unwrap_or(0) as usize, &mut self.ota_handle);
                            if res != ESP_OK {
                                self.ota_state = OtaState::Failed(format!("Failed to begin OTA: {}", res));
                                result = Err(anyhow!("Failed to begin OTA: {}", res));
                            } else {
                                for i in 0..3 {
                                    if let Err(e) = self.request_firmware_chunk(mqtt_client, self.current_chunk + i) {
                                        self.ota_state = OtaState::Failed(format!("Failed to request firmware chunk: {}", e));
                                        result = Err(e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if let Err(e) = self.send_ota_telemetry(mqtt_client) {
                        error!("Failed to send OTA telemetry: {:?}", e);
                    }
                }
            } else {
                info!("No new firmware detected: title and version match current");
            }
        } else {
            info!("Incomplete firmware attributes: fw_title={:?}, fw_version={:?}", self.fw_title, self.fw_version);
            result = Err(anyhow!("Incomplete firmware attributes received"));
        }
        result
    }   

    fn request_firmware_info(&mut self, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        self.request_id += 1;
        let request_topic = format!("{}{}", OTA_REQUEST_TOPIC, self.request_id);
        let payload = json!({
            "sharedKeys": format!("{},{},{},{},{}",
                FW_TITLE_ATTR, FW_VERSION_ATTR, FW_SIZE_ATTR, FW_CHECKSUM_ATTR, FW_CHECKSUM_ALG_ATTR)
        });
        Self::mqtt_publish(mqtt_client, &request_topic, &payload.to_string())?;
        info!("Requested firmware info, topic: {}", request_topic);
        Ok(())
    }

    fn request_firmware_chunk(&mut self, mqtt_client: *mut esp_mqtt_client, chunk_index: u32) -> Result<()> {
        if let Some(fw_size) = self.fw_size {
            if self.received_size >= fw_size as usize {
                info!("All firmware chunks received, no further requests needed");
                return Ok(());
            }
        }
        let topic = format!("{}/{}/chunk/{}", OTA_FIRMWARE_REQUEST_TOPIC, self.firmware_request_id, chunk_index);
        let payload = self.chunk_size.to_string();
        Self::mqtt_publish(mqtt_client, &topic, &payload)?;
        info!("Requested firmware chunk {}, topic: {}", chunk_index, topic);
        Ok(())
    }

    fn handle_firmware_chunk(&mut self, data: &[u8], chunk_index: u32, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        if chunk_index == self.current_chunk {
            if data.len() == 0 {
                if self.received_size == self.fw_size.unwrap_or(0) as usize {
                    info!("Received empty chunk, download complete");
                    self.ota_state = OtaState::Downloaded;
                    unsafe {
                        let res = esp_ota_end(self.ota_handle);
                        if res != ESP_OK {
                            self.ota_state = OtaState::Failed(format!("Failed to end OTA: {}", res));
                            self.send_ota_telemetry(mqtt_client)?;
                            return Err(anyhow!("Failed to end OTA: {}", res));
                        }
                    }
                    self.process_firmware(mqtt_client)?;
                    return Ok(());
                } else {
                    self.ota_state = OtaState::Failed("Received empty chunk but size mismatch".to_string());
                    self.send_ota_telemetry(mqtt_client)?;
                    return Err(anyhow!("Empty chunk received prematurely"));
                }
            }

            self.received_size += data.len();
            info!("Received chunk {}, size: {}, total received: {}", chunk_index, data.len(), self.received_size);
            
            if let Some(fw_size) = self.fw_size {
                let percentage = (self.received_size as f32 / fw_size as f32) * 100.0;
                info!("Download progress: {:.2}% ({} / {})", percentage, self.received_size, fw_size);
            }
            
            self.sha256_hasher.update(data);
            unsafe {
                let res = esp_ota_write(self.ota_handle, data.as_ptr() as *const c_void, data.len());
                if res != ESP_OK {
                    self.ota_state = OtaState::Failed(format!("Failed to write OTA data: {}", res));
                    self.send_ota_telemetry(mqtt_client)?;
                    return Err(anyhow!("Failed to write OTA data: {}", res));
                }
            }

            self.current_chunk += 1;
            self.last_chunk_received = unsafe { xTaskGetTickCount() };
            if let Some(fw_size) = self.fw_size {
                if self.received_size >= fw_size as usize {
                    self.ota_state = OtaState::Downloaded;
                    unsafe {
                        let res = esp_ota_end(self.ota_handle);
                        if res != ESP_OK {
                            self.ota_state = OtaState::Failed(format!("Failed to end OTA: {}", res));
                            self.send_ota_telemetry(mqtt_client)?;
                            return Err(anyhow!("Failed to end OTA: {}", res));
                        }
                    }
                    self.process_firmware(mqtt_client)?;
                } else {
                    self.process_buffered_chunks(mqtt_client)?;
                    self.request_firmware_chunk(mqtt_client, self.current_chunk)?;
                }
            }
            Ok(())
        } else {
            info!("Received out-of-order chunk {}, storing in buffer", chunk_index);
            self.chunk_buffer.push((chunk_index, data.to_vec()));
            self.chunk_buffer.sort_by_key(|&(index, _)| index);
            self.process_buffered_chunks(mqtt_client)?;
            Ok(())
        }
    }

    fn process_buffered_chunks(&mut self, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        while let Some(index) = self.chunk_buffer.iter().position(|&(index, _)| index == self.current_chunk) {
            let (_, data) = self.chunk_buffer.remove(index);
            self.handle_firmware_chunk(&data, self.current_chunk, mqtt_client)?;
        }
        Ok(())
    }

    fn process_firmware(&mut self, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        self.ota_state = OtaState::Verifying;
        self.send_ota_telemetry(mqtt_client)?;

        if let Some(checksum) = &self.fw_checksum {
            let computed_checksum = {
                let result = self.sha256_hasher.clone().finalize();
                result.iter().map(|b| format!("{:02x}", b)).collect::<String>()
            };
            info!("Computed checksum: {}, Expected checksum: {}", computed_checksum, checksum);
            if computed_checksum == *checksum {
                self.ota_state = OtaState::Updating;
                self.send_ota_telemetry(mqtt_client)?;
                unsafe {
                    let res = esp_ota_set_boot_partition(self.ota_partition);
                    if res != ESP_OK {
                        self.ota_state = OtaState::Failed(format!("Failed to set boot partition: {}", res));
                        self.send_ota_telemetry(mqtt_client)?;
                        return Err(anyhow!("Failed to set boot partition: {}", res));
                    }
                }
                self.current_fw_title = self.fw_title.clone().unwrap_or_default();
                self.current_fw_version = self.fw_version.clone().unwrap_or_default();
                self.ota_state = OtaState::Updated;
                self.send_ota_telemetry(mqtt_client)?;
                info!("Firmware update successful, restarting...");
                unsafe { esp_restart(); }
            } else {
                self.ota_state = OtaState::Failed("Checksum verification failed".to_string());
                self.send_ota_telemetry(mqtt_client)?;
                return Err(anyhow!("Checksum verification failed"));
            }
        } else {
            self.ota_state = OtaState::Failed("No checksum provided".to_string());
            self.send_ota_telemetry(mqtt_client)?;
            return Err(anyhow!("No checksum provided"));
        }
    }

    fn send_ota_telemetry(&mut self, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        self.telemetry_counter += 1;
        if self.ota_state == OtaState::Downloading && self.telemetry_counter < ms_to_ticks(5000) / ms_to_ticks(100) {
            return Ok(());
        }
        self.telemetry_counter = 0;
        let payload = match &self.ota_state {
            OtaState::Idle => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "IDLE"
            }).to_string(),
            OtaState::Downloading => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "DOWNLOADING",
                "progress": if let Some(fw_size) = self.fw_size { self.received_size as f32 / fw_size as f32 * 100.0 } else { 0.0 }
            }).to_string(),
            OtaState::Downloaded => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "DOWNLOADED"
            }).to_string(),
            OtaState::Verifying => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "VERIFYING"
            }).to_string(),
            OtaState::Updating => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "UPDATING"
            }).to_string(),
            OtaState::Updated => json!({
                "current_fw_title": &self.current_fw_title,
                "current_fw_version": &self.current_fw_version,
                FW_STATE_ATTR: "UPDATED"
            }).to_string(),
            OtaState::Failed(error) => json!({
                FW_STATE_ATTR: "FAILED",
                "fw_error": error
            }).to_string(),
        };
        Self::mqtt_publish(mqtt_client, OTA_TELEMETRY_TOPIC, &payload)?;
        info!("Sent OTA telemetry: {}", payload);
        Ok(())
    }

    fn check_chunk_timeout(&mut self, mqtt_client: *mut esp_mqtt_client) -> Result<()> {
        if self.ota_state == OtaState::Downloading {
            let current_ticks = unsafe { xTaskGetTickCount() };
            if current_ticks - self.last_chunk_received > ms_to_ticks(10000) {
                info!("No chunks received for 10 seconds, re-requesting chunk {}", self.current_chunk);
                self.request_firmware_chunk(mqtt_client, self.current_chunk)?;
                self.last_chunk_received = current_ticks;
            }
        }
        Ok(())
    }

    fn mqtt_publish(mqtt_client: *mut esp_mqtt_client, topic: &str, data: &str) -> Result<()> {
        unsafe {
            let topic_cstr = CString::new(topic)?;
            let data_cstr = CString::new(data)?;
            let msg_id = esp_mqtt_client_publish(
                mqtt_client,
                topic_cstr.as_ptr(),
                data_cstr.as_ptr(),
                data.len() as i32,
                1,
                0
            );
            if msg_id < 0 {
                Err(anyhow!("Failed to publish message to {}: {}", topic, msg_id))
            } else {
                info!("Published message to {} with ID: {}", topic, msg_id);
                Ok(())
            }
        }
    }
}

struct SimpleMqttClient {
    client: *mut esp_mqtt_client,
}

impl SimpleMqttClient {
    fn new(broker_url: &str, username: &str, password: &str, client_id: &str, ota_manager_ptr: *mut OtaManager) -> Result<Self> {
        unsafe {
            let broker_url_cstr = CString::new(broker_url)?;
            let username_cstr = CString::new(username)?;
            let password_cstr = CString::new(password)?;
            let client_id_cstr = CString::new(client_id)?;
            let config = esp_mqtt_client_config_t {
                broker: esp_mqtt_client_config_t_broker_t {
                    address: esp_mqtt_client_config_t_broker_t_address_t {
                        uri: broker_url_cstr.as_ptr(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                credentials: esp_mqtt_client_config_t_credentials_t {
                    username: username_cstr.as_ptr(),
                    client_id: client_id_cstr.as_ptr(),
                    authentication: esp_mqtt_client_config_t_credentials_t_authentication_t {
                        password: password_cstr.as_ptr(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                buffer: esp_mqtt_client_config_t_buffer_t {
                    size: 8192,
                    out_size: 8192,
                    ..Default::default()
                },
                ..Default::default()
            };
            let client = esp_mqtt_client_init(&config);
            if client.is_null() {
                return Err(anyhow!("Failed to initialize MQTT client"));
            }
            esp_mqtt_client_register_event(
                client,
                esp_mqtt_event_id_t_MQTT_EVENT_ANY,
                Some(Self::mqtt_event_handler),
                ota_manager_ptr as *mut c_void
            );
            let err = esp_mqtt_client_start(client);
            if err != ESP_OK {
                esp_mqtt_client_destroy(client);
                return Err(anyhow!("Failed to start MQTT client, error code: {}", err));
            }
            vTaskDelay(ms_to_ticks(5000));
            info!("MQTT client started, verifying subscriptions...");
            Ok(Self { client })
        }
    }

    extern "C" fn mqtt_event_handler(
        handler_args: *mut c_void,
        _base: *const u8,
        event_id: i32,
        event_data: *mut c_void
    ) {
        unsafe {
            let ota_manager = handler_args as *mut OtaManager;
            if ota_manager.is_null() {
                error!("OTA manager pointer is null");
                return;
            }
            let event = &*(event_data as *mut esp_mqtt_event_t);
            info!("MQTT event received, event_id: {}", event_id);
            match event_id {
                id if id == esp_mqtt_event_id_t_MQTT_EVENT_CONNECTED as i32 => {
                    info!("MQTT connected to broker");
                }
                id if id == esp_mqtt_event_id_t_MQTT_EVENT_DISCONNECTED as i32 => {
                    error!("MQTT disconnected from broker");
                }
                id if id == esp_mqtt_event_id_t_MQTT_EVENT_SUBSCRIBED as i32 => {
                    let topic_len = event.topic_len as usize;
                    let topic = if topic_len > 0 {
                        let topic_slice = core::slice::from_raw_parts(event.topic as *const u8, topic_len);
                        core::str::from_utf8(topic_slice).unwrap_or("unknown")
                    } else {
                        "unknown"
                    };
                    info!("Subscribed to topic: {}", topic);
                }
                id if id == esp_mqtt_event_id_t_MQTT_EVENT_DATA as i32 => {
                    let topic_len = event.topic_len as usize;
                    let data_len = event.data_len as usize;
                    if topic_len > 0 && data_len > 0 {
                        let topic_slice = core::slice::from_raw_parts(event.topic as *const u8, topic_len);
                        let topic = core::str::from_utf8(topic_slice).unwrap_or("");
                        info!("Received MQTT message on topic: {}, data_len: {}", topic, data_len);
                        let data_slice = core::slice::from_raw_parts(event.data as *const u8, data_len);
                        if topic.starts_with(OTA_RESPONSE_TOPIC) {
                            if let Ok(data_str) = core::str::from_utf8(data_slice) {
                                info!("OTA response data: {}", data_str);
                                if let Err(e) = (*ota_manager).handle_shared_attributes(data_str, event.client) {
                                    error!("Failed to handle OTA attributes: {:?}", e);
                                }
                            } else {
                                error!("Invalid UTF-8 in OTA response");
                            }
                        } else if topic.starts_with(&format!("{}/{}/", OTA_FIRMWARE_RESPONSE_TOPIC, (*ota_manager).firmware_request_id)) {
                            let total_len = event.total_data_len as usize;
                            let offset = event.current_data_offset as usize;
                            let chunk_data_len = event.data_len as usize;
                            let data_slice = core::slice::from_raw_parts(event.data as *const u8, chunk_data_len);

                            if offset == 0 {
                                (*ota_manager).partial_firmware_data.clear();
                                (*ota_manager).partial_firmware_data.extend_from_slice(data_slice);
                            } else {
                                (*ota_manager).partial_firmware_data.extend_from_slice(data_slice);
                            }

                            if offset + chunk_data_len >= total_len {
                                let topic_parts: Vec<&str> = topic.split('/').collect();
                                if let Some(chunk_str) = topic_parts.last() {
                                    if let Ok(chunk_index) = chunk_str.parse::<u32>() {
                                        info!("Received complete firmware chunk for request ID: {}, chunk: {}, data length: {}", 
                                            (*ota_manager).firmware_request_id, chunk_index, (*ota_manager).partial_firmware_data.len());
                                        (*ota_manager).last_chunk_received = xTaskGetTickCount();
                                        if let Err(e) = (*ota_manager).handle_firmware_chunk(&(*ota_manager).partial_firmware_data, chunk_index, event.client) {
                                            error!("Failed to handle firmware chunk: {:?}", e);
                                        }
                                    } else {
                                        error!("Invalid chunk index in topic: {}", topic);
                                    }
                                }
                                (*ota_manager).partial_firmware_data.clear();
                            }
                        } else {
                            info!("Received MQTT message on unexpected topic: {}", topic);
                        }
                    }
                }
                _ => {
                    info!("Unhandled MQTT event, event_id: {}", event_id);
                }
            }
        }
    }

    fn publish(&self, topic: &str, data: &str) -> Result<()> {
        OtaManager::mqtt_publish(self.client, topic, data)
    }

    fn subscribe(&self, topic: &str) -> Result<()> {
        unsafe {
            let topic_cstr = CString::new(topic)?;
            let result = esp_mqtt_client_subscribe_single(
                self.client,
                topic_cstr.as_ptr(),
                1
            );
            if result == -1 {
                error!("Failed to subscribe to topic: {}, retrying...", topic);
                vTaskDelay(ms_to_ticks(1000));
                let retry_result = esp_mqtt_client_subscribe_single(
                    self.client,
                    topic_cstr.as_ptr(),
                    1
                );
                if retry_result == -1 {
                    Err(anyhow!("Failed to subscribe to topic after retry: {}", topic))
                } else {
                    info!("Subscribed to topic after retry: {}", topic);
                    Ok(())
                }
            } else {
                info!("Subscribed to topic: {}", topic);
                Ok(())
            }
        }
    }
}

impl Drop for SimpleMqttClient {
    fn drop(&mut self) {
        unsafe {
            esp_mqtt_client_stop(self.client);
            esp_mqtt_client_destroy(self.client);
        }
    }
}

fn send_telemetry(
    mqtt_client: &SimpleMqttClient,
    temperature: f32,
    humidity: f32,
    pressure: f32,
    co2_ppm: f32
) -> Result<()> {
    let payload = json!({
        "temperature": temperature,
        "humidity": humidity,
        "pressure": pressure / 100.0,
        "co2_ppm": co2_ppm,
        "latitude": -7.278306,
        "longitude": 112.792028
    }).to_string();
    mqtt_client.publish(OTA_TELEMETRY_TOPIC, &payload)?;
    info!("Data sent to ThingsBoard: {}", payload);
    Ok(())
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    let ssid = "GRATIS";
    let password = "Gakgratis";
    let wifi_config = Configuration::Client(ClientConfiguration {
        ssid: heapless::String::try_from(ssid).unwrap(),
        password: heapless::String::try_from(password).unwrap(),
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    });
    wifi.set_configuration(&wifi_config)?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;
    let ip_info: IpInfo = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi Connected, IP: {}", ip_info.ip);
    Ok(())
}

#[no_mangle]
fn main() -> i32 {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Starting BME280 + WiFi + CO2 ADC + MQTT application");

    let peripherals = Peripherals::take().unwrap();
    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs)).unwrap(),
        sys_loop,
    ).unwrap();

    if let Err(e) = connect_wifi(&mut wifi) {
        error!("Failed to connect to WiFi: {:?}", e);
        return -1;
    }

    let scl = peripherals.pins.gpio9;
    let sda = peripherals.pins.gpio8;
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        sda,
        scl,
        &esp_idf_hal::i2c::I2cConfig::new().baudrate(100.kHz().into())
    ).unwrap();
    let mut bme280 = BME280::new_primary(i2c);
    let mut delay = Ets;

    if let Err(e) = bme280.init(&mut delay) {
        error!("Failed to init BME280: {:?}", e);
        return -1;
    }

    info!("Connecting to MQTT broker...");
    let mut ota_manager = Box::new(OtaManager::new());
    let ota_manager_ptr = &mut *ota_manager as *mut OtaManager;

    let mqtt_client = match SimpleMqttClient::new(
        "mqtt://mqtt.thingsboard.cloud:1883",
        "nazwana",
        "akuandik08",
        "eprtrartn5tpdw7oq38f",
        ota_manager_ptr
    ) {
        Ok(client) => {
            info!("Connected to ThingsBoard MQTT broker");
            if let Err(e) = client.subscribe("v1/devices/me/attributes/response/+") {
                error!("Failed to subscribe to OTA response: {:?}", e);
            }
            if let Err(e) = client.subscribe("v1/devices/me/attributes") {
                error!("Failed to subscribe to attributes: {:?}", e);
            }
            if let Err(e) = client.subscribe("v2/fw/response/+/chunk/+") {
                error!("Failed to subscribe to firmware response: {:?}", e);
            }
            client
        },
        Err(e) => {
            error!("Failed to connect to MQTT: {:?}", e);
            return -1;
        }
    };

    if let Err(e) = ota_manager.request_firmware_info(mqtt_client.client) {
        error!("Failed to request firmware info: {:?}", e);
    }

    unsafe {
        let init_cfg = adc_oneshot_unit_init_cfg_t {
            unit_id: adc_unit_t_ADC_UNIT_2,
            clk_src: soc_periph_adc_rtc_clk_src_t_ADC_RTC_CLK_SRC_DEFAULT,
            ..Default::default()
        };
        let mut adc2_handle: adc_oneshot_unit_handle_t = core::ptr::null_mut();
        let res = adc_oneshot_new_unit(&init_cfg, &mut adc2_handle);
        if res != ESP_OK {
            error!("Failed to init ADC unit");
            return -1;
        }

        let chan_cfg = adc_oneshot_chan_cfg_t {
            atten: adc_atten_t_ADC_ATTEN_DB_11,
            bitwidth: adc_bitwidth_t_ADC_BITWIDTH_DEFAULT,
        };
        let res = adc_oneshot_config_channel(
            adc2_handle,
            adc_channel_t_ADC_CHANNEL_1,
            &chan_cfg,
        );
        if res != ESP_OK {
            error!("Failed to config ADC channel");
            return -1;
        }

        let mut counter = 0;
        let mut ota_check_counter = 0;
        loop {
            counter += 1;
            ota_check_counter += 1;

            if ota_manager.ota_state == OtaState::Downloading {
                if let Err(e) = ota_manager.check_chunk_timeout(mqtt_client.client) {
                    error!("Failed to check chunk timeout: {:?}", e);
                }
                if ota_manager.telemetry_counter == 0 {
                    let measurements = match bme280.measure(&mut delay) {
                        Ok(m) => m,
                        Err(e) => {
                            error!("BME280 read error: {:?}", e);
                            vTaskDelay(ms_to_ticks(1000));
                            continue;
                        }
                    };

                    let mut value: i32 = 0;
                    let res = adc_oneshot_read(adc2_handle, adc_channel_t_ADC_CHANNEL_1, &mut value);
                    let co2_ppm = if res == ESP_OK {
                        adc_to_ppm(value)
                    } else {
                        error!("ADC read error");
                        0.0
                    };

                    info!("=== Reading {} ===", counter);
                    info!("Temperature: {:.2} °C", measurements.temperature);
                    info!("Humidity: {:.2} %", measurements.humidity);
                    info!("Pressure: {:.2} hPa", measurements.pressure / 100.0);
                    info!("CO2 Concentration: {:.2} ppm", co2_ppm);

                    if let Err(e) = send_telemetry(
                        &mqtt_client,
                        measurements.temperature,
                        measurements.humidity,
                        measurements.pressure,
                        co2_ppm
                    ) {
                        error!("Failed to send telemetry: {:?}", e);
                    }
                }
                vTaskDelay(ms_to_ticks(100));
            } else {
                if ota_check_counter >= 6 {
                    ota_check_counter = 0;
                    if let Err(e) = ota_manager.request_firmware_info(mqtt_client.client) {
                        error!("Failed to request firmware info: {:?}", e);
                    }
                }

                let measurements = match bme280.measure(&mut delay) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("BME280 read error: {:?}", e);
                        vTaskDelay(ms_to_ticks(1000));
                        continue;
                    }
                };

                let mut value: i32 = 0;
                let res = adc_oneshot_read(adc2_handle, adc_channel_t_ADC_CHANNEL_1, &mut value);
                let co2_ppm = if res == ESP_OK {
                    adc_to_ppm(value)
                } else {
                    error!("ADC read error");
                    0.0
                };

                info!("=== Reading {} ===", counter);
                info!("Temperature: {:.2} °C", measurements.temperature);
                info!("Humidity: {:.2} %", measurements.humidity);
                info!("Pressure: {:.2} hPa", measurements.pressure / 100.0);
                info!("CO2 Concentration: {:.2} ppm", co2_ppm);

                if let Err(e) = send_telemetry(
                    &mqtt_client,
                    measurements.temperature,
                    measurements.humidity,
                    measurements.pressure,
                    co2_ppm
                ) {
                    error!("Failed to send telemetry: {:?}", e);
                }

                vTaskDelay(ms_to_ticks(5000));
            }

            if ota_manager.ota_state != OtaState::Idle {
                if let Err(e) = ota_manager.send_ota_telemetry(mqtt_client.client) {
                    error!("Failed to send OTA telemetry: {:?}", e);
                }
            }
        }
    }
}

// V2.0

// #![no_std]
// #![no_main]

// use esp_idf_sys::*;
// use esp_idf_hal::{delay::Ets, i2c::I2cDriver, peripherals::Peripherals, prelude::*};
// use esp_idf_svc::{
//     eventloop::EspSystemEventLoop,
//     nvs::EspDefaultNvsPartition,
//     wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
//     ipv4::IpInfo,
//     sntp::{EspSntp, SyncStatus},
// };
// use bme280::i2c::BME280;
// use log::{info, error};
// use anyhow::Result;
// use serde_json::json;
// use alloc::string::{String, ToString};
// use alloc::format;
// use alloc::ffi::CString;

// extern crate alloc;

// #[inline(always)]
// fn ms_to_ticks(ms: u32) -> u32 {
//     (ms as u64 * configTICK_RATE_HZ as u64 / 1000) as u32
// }

// fn adc_to_ppm(adc_raw: i32) -> f32 {
//     let adc_min = 0.0;
//     let adc_max = 3500.0;
//     let ppm_min = 0.0;
//     let ppm_max = 1200.0;

//     let adc_f = adc_raw as f32;
//     let ppm = (adc_max - adc_f) / (adc_max - adc_min) * (ppm_max - ppm_min) + ppm_min;

//     if ppm < ppm_min {
//         ppm_min
//     } else if ppm > ppm_max {
//         ppm_max
//     } else {
//         ppm
//     }
// }

// struct SimpleMqttClient {
//     client: *mut esp_mqtt_client,
// }

// impl SimpleMqttClient {
//     fn new(broker_url: &str, username: &str, password: &str, client_id: &str) -> Result<Self> {
//         unsafe {
//             let broker_url_cstr = CString::new(broker_url)?;
//             let username_cstr = CString::new(username)?;
//             let password_cstr = CString::new(password)?;
//             let client_id_cstr = CString::new(client_id)?;

//             let config = esp_mqtt_client_config_t {
//                 broker: esp_mqtt_client_config_t_broker_t {
//                     address: esp_mqtt_client_config_t_broker_t_address_t {
//                         uri: broker_url_cstr.as_ptr(),
//                         ..core::mem::zeroed()
//                     },
//                     ..core::mem::zeroed()
//                 },
//                 credentials: esp_mqtt_client_config_t_credentials_t {
//                     username: username_cstr.as_ptr(),
//                     client_id: client_id_cstr.as_ptr(),
//                     authentication: esp_mqtt_client_config_t_credentials_t_authentication_t {
//                         password: password_cstr.as_ptr(),
//                         ..core::mem::zeroed()
//                     },
//                     ..core::mem::zeroed()
//                 },
//                 buffer: esp_mqtt_client_config_t_buffer_t {
//                     size: 8192,
//                     out_size: 8192,
//                     ..Default::default()
//                 },
//                 ..core::mem::zeroed()
//             };

//             let client = esp_mqtt_client_init(&config);
//             if client.is_null() {
//                 return Err(anyhow::anyhow!("Failed to initialize MQTT client"));
//             }

//             let err = esp_mqtt_client_start(client);
//             if err != ESP_OK {
//                 esp_mqtt_client_destroy(client);
//                 return Err(anyhow::anyhow!("Failed to start MQTT client, error code: {}", err));
//             }

//             vTaskDelay(ms_to_ticks(5000));
//             info!("MQTT client started");
//             Ok(Self { client })
//         }
//     }

//     fn publish(&self, topic: &str, data: &str) -> Result<()> {
//         unsafe {
//             let topic_cstr = CString::new(topic)?;
//             let data_cstr = CString::new(data)?;

//             let msg_id = esp_mqtt_client_publish(
//                 self.client,
//                 topic_cstr.as_ptr(),
//                 data_cstr.as_ptr(),
//                 data.len() as i32,
//                 1,
//                 0,
//             );

//             if msg_id < 0 {
//                 Err(anyhow::anyhow!("Failed to publish message to {}, error code: {}", topic, msg_id))
//             } else {
//                 info!("Published message to {} with ID: {}", topic, msg_id);
//                 Ok(())
//             }
//         }
//     }
// }

// impl Drop for SimpleMqttClient {
//     fn drop(&mut self) {
//         unsafe {
//             esp_mqtt_client_stop(self.client);
//             esp_mqtt_client_destroy(self.client);
//         }
//     }
// }

// fn get_rtc_timestamp() -> Result<String> {
//     unsafe {
//         let mut tv: timeval = core::mem::zeroed();
//         let ret = gettimeofday(&mut tv, core::ptr::null_mut());
//         if ret != 0 {
//             return Err(anyhow::anyhow!("Failed to get RTC time"));
//         }

//         // Adjust for UTC+7 (WIB) by adding 7 hours (25,200 seconds)
//         let seconds = tv.tv_sec + 25_200;
//         let mut tm: tm = core::mem::zeroed();
//         let tm_ptr = gmtime_r(&seconds, &mut tm);
//         if tm_ptr.is_null() {
//             return Err(anyhow::anyhow!("Failed to convert time to UTC"));
//         }

//         let local_str = format!(
//             "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
//             tm.tm_year + 1900,
//             tm.tm_mon + 1,
//             tm.tm_mday,
//             tm.tm_hour,
//             tm.tm_min,
//             tm.tm_sec
//         );

//         Ok(local_str)
//     }
// }

// fn send_telemetry(
//     mqtt_client: &SimpleMqttClient,
//     temperature: f32,
//     humidity: f32,
//     pressure: f32,
//     co2_ppm: f32,
//     is_initial: bool,
//     timestamp: &str,
// ) -> Result<()> {
//     let payload = if is_initial {
//         json!({
//             "temperature": temperature,
//             "humidity": humidity,
//             "pressure": pressure / 100.0,
//             "co2_ppm": co2_ppm,
//             "latitude": -7.278306,
//             "longitude": 112.792028,
//             "current_fw_title": "Weather Station",
//             "current_fw_version": "V2.0",
//             "fw_state": "UPDATED",
//             "sensor_timestamp": timestamp
//         }).to_string()
//     } else {
//         json!({
//             "temperature": temperature,
//             "humidity": humidity,
//             "pressure": pressure / 100.0,
//             "co2_ppm": co2_ppm,
//             "latitude": -7.278306,
//             "longitude": 112.792028,
//             "sensor_timestamp": timestamp
//         }).to_string()
//     };

//     mqtt_client.publish("v1/devices/me/telemetry", &payload)?;
//     info!("Data sent to ThingsBoard: {}", payload);
//     Ok(())
// }

// fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
//     let ssid = "GRATIS";
//     let password = "Gakgratis";
//     let wifi_config = Configuration::Client(ClientConfiguration {
//         ssid: heapless::String::try_from(ssid).unwrap(),
//         password: heapless::String::try_from(password).unwrap(),
//         auth_method: AuthMethod::WPA2Personal,
//         ..Default::default()
//     });
//     wifi.set_configuration(&wifi_config)?;
//     wifi.start()?;
//     wifi.connect()?;
//     wifi.wait_netif_up()?;
//     let ip_info: IpInfo = wifi.wifi().sta_netif().get_ip_info()?;
//     info!("WiFi Connected, IP: {}", ip_info.ip);
//     Ok(())
// }

// fn init_sntp() -> Result<()> {
//     let sntp = EspSntp::new_default()?;
//     info!("SNTP initialized, waiting for sync...");

//     // Wait for SNTP sync
//     for _ in 0..30 {
//         if sntp.get_sync_status() == SyncStatus::Completed {
//             info!("SNTP sync completed");
//             core::mem::forget(sntp);
//             return Ok(());
//         }
//         unsafe { vTaskDelay(ms_to_ticks(1000)); }
//     }
//     Err(anyhow::anyhow!("SNTP sync timed out"))
// }

// #[no_mangle]
// fn main() -> i32 {
//     esp_idf_sys::link_patches();
//     esp_idf_svc::log::EspLogger::initialize_default();
//     info!("Starting BME280 + WiFi + CO2 ADC + MQTT + RTC application");

//     let peripherals = Peripherals::take().unwrap();
//     let sys_loop = EspSystemEventLoop::take().unwrap();
//     let nvs = EspDefaultNvsPartition::take().unwrap();

//     let mut wifi = BlockingWifi::wrap(
//         EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs)).unwrap(),
//         sys_loop,
//     ).unwrap();

//     if let Err(e) = connect_wifi(&mut wifi) {
//         error!("Failed to connect to WiFi: {:?}", e);
//         return -1;
//     }

//     // Initialize SNTP for RTC synchronization
//     if let Err(e) = init_sntp() {
//         error!("Failed to initialize SNTP: {:?}", e);
//         return -1;
//     }

//     let scl = peripherals.pins.gpio9;
//     let sda = peripherals.pins.gpio8;
//     let i2c = I2cDriver::new(
//         peripherals.i2c0,
//         sda,
//         scl,
//         &esp_idf_hal::i2c::I2cConfig::new().baudrate(100.kHz().into())
//     ).unwrap();

//     let mut bme280 = BME280::new_primary(i2c);
//     let mut delay = Ets;
//     if let Err(e) = bme280.init(&mut delay) {
//         error!("Failed to init BME280: {:?}", e);
//         return -1;
//     }

//     info!("Connecting to MQTT broker...");
//     let mqtt_client = match SimpleMqttClient::new(
//         "mqtt://mqtt.thingsboard.cloud:1883",
//         "nazwana",
//         "akuandik08",
//         "eprtrartn5tpdw7oq38f"
//     ) {
//         Ok(client) => {
//             info!("Connected to ThingsBoard MQTT broker");
//             client
//         },
//         Err(e) => {
//             error!("Failed to connect to MQTT: {:?}", e);
//             return -1;
//         }
//     };

//     unsafe {
//         let init_cfg = adc_oneshot_unit_init_cfg_t {
//             unit_id: adc_unit_t_ADC_UNIT_2,
//             clk_src: soc_periph_adc_rtc_clk_src_t_ADC_RTC_CLK_SRC_DEFAULT,
//             ..Default::default()
//         };
//         let mut adc2_handle: adc_oneshot_unit_handle_t = core::ptr::null_mut();
//         let res = adc_oneshot_new_unit(&init_cfg, &mut adc2_handle);
//         if res != ESP_OK {
//             error!("Failed to init ADC unit");
//             return -1;
//         }

//         let chan_cfg = adc_oneshot_chan_cfg_t {
//             atten: adc_atten_t_ADC_ATTEN_DB_11,
//             bitwidth: adc_bitwidth_t_ADC_BITWIDTH_DEFAULT,
//         };
//         let res = adc_oneshot_config_channel(
//             adc2_handle,
//             adc_channel_t_ADC_CHANNEL_1,
//             &chan_cfg,
//         );
//         if res != ESP_OK {
//             error!("Failed to config ADC channel");
//             return -1;
//         }

//         let mut counter = 0;
//         let mut is_initial = true;
//         loop {
//             counter += 1;

//             let measurements = match bme280.measure(&mut delay) {
//                 Ok(m) => m,
//                 Err(e) => {
//                     error!("BME280 read error: {:?}", e);
//                     vTaskDelay(ms_to_ticks(1000));
//                     continue;
//                 }
//             };

//             let mut value: i32 = 0;
//             let res = adc_oneshot_read(adc2_handle, adc_channel_t_ADC_CHANNEL_1, &mut value);
//             let co2_ppm = if res == ESP_OK {
//                 adc_to_ppm(value)
//             } else {
//                 error!("ADC read error");
//                 0.0
//             };

//             // Get RTC timestamp before sending telemetry
//             let timestamp = match get_rtc_timestamp() {
//                 Ok(ts) => ts,
//                 Err(e) => {
//                     error!("Failed to get RTC timestamp: {:?}", e);
//                     String::from("1970-01-01 00:00:00") // Fallback timestamp
//                 }
//             };

//             info!("=== Reading {} ===", counter);
//             info!("Temperature: {:.2} °C", measurements.temperature);
//             info!("Humidity: {:.2} %", measurements.humidity);
//             info!("Pressure: {:.2} hPa", measurements.pressure / 100.0);
//             info!("CO2 Concentration: {:.2} ppm", co2_ppm);
//             info!("Sensor Timestamp: {}", timestamp);

//             if let Err(e) = send_telemetry(
//                 &mqtt_client,
//                 measurements.temperature,
//                 measurements.humidity,
//                 measurements.pressure,
//                 co2_ppm,
//                 is_initial,
//                 &timestamp,
//             ) {
//                 error!("Failed to send telemetry: {:?}", e);
//             }

//             is_initial = false; // Only send initial telemetry once
//             vTaskDelay(ms_to_ticks(5000));
//         }
//     }
// }