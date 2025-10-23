# **IoT Environmental Monitoring System with ESP32-S3 & OTA Firmware Update**  
### *Internet of Things (IoT) Course Project*  
**Department of Instrumentation Engineering, Vocational Faculty – Institut Teknologi Sepuluh Nopember (ITS)**  

---

## **Developers**
| **Supervisor** | **Students** |
|----------------|--------------|
| **Ahmad Radhy, S.Si., M.Si.** | **Andik Putra Nazwana**<br>**Rany Surya Oktavia** |

---

## **Abstract**
This IoT system enables **real-time environmental monitoring** using the **ESP32-S3** microcontroller programmed in **Rust `no_std`** — highly optimized for embedded systems. Sensor data from **BME280** (temperature, humidity, pressure) and **MQ-135** (CO₂ concentration) is transmitted to **ThingsBoard Cloud** via **MQTT**.

Key feature: **Over-the-Air (OTA) Firmware Updates** with **dual-partition safety**, **SHA256 checksum verification**, and **automatic rollback** — no physical access required.

---

## **Key Features**

| Feature | Description |
|-------|-----------|
| **Real-time Telemetry** | Temperature, humidity, pressure, CO₂, GPS location (static) |
| **OTA Firmware Update** | Remote firmware deployment via ThingsBoard |
| **Dual OTA Partitions** | `ota_0` & `ota_1` for safe updates & rollback |
| **Checksum Verification** | SHA256 validation before boot |
| **Dual Dashboards** | 1. Sensor data, 2. OTA status |
| **Rust `no_std`** | Minimal memory footprint & memory safety |
| **RTC Timestamp (V2.0)** | End-to-end latency measurement |
| **SNTP Sync** | Accurate time via internet |

---

## **System Architecture**

```
[ESP32-S3 + Sensors]
        ↓ (I²C & ADC)
[BME280 + MQ-135]
        ↓ (WiFi + MQTT)
[ThingsBoard Cloud]
        ↓
[Dashboards: Data + OTA Status]
```

---

## **Hardware Components**

| Component | Function |
|---------|--------|
| **ESP32-S3** | Main MCU (WiFi, dual-core, rich peripherals) |
| **BME280** | Environmental sensor (I²C): temp, humidity, pressure |
| **MQ-135** | Gas sensor (ADC): CO₂, air quality |
| **WiFi Antenna** | Network connectivity |

---

## **Software Stack**

- **Language**: Rust `no_std`
- **Framework**: `esp-idf-hal`, `esp-idf-svc`
- **IoT Platform**: [ThingsBoard Cloud](https://thingsboard.cloud)
- **Protocol**: MQTT
- **OTA Service**: ThingsBoard Firmware OTA
- **Build Tools**: `cargo`, `espflash`, `embuild`

---

## **Flash Partition Table (`partitions.csv`)**

```csv
# Name,Type,SubType,Offset,Size
nvs,data,nvs,0x9000,0x4000
otadata,data,ota,0xd000,0x2000
phy_init,data,phy,0xf000,0x1000
ota_0,app,ota_0,0x10000,0x600000
ota_1,app,ota_1,0x610000,0x600000
spiffs,data,spiffs,0xc10000,0x3f0000
```

> **Dual OTA Slots**: Ensures safe updates with rollback capability.

---

## **Firmware Versions**

| Version | Key Features |
|--------|-------------|
| **V1.0** (`main.rs`) | Sensor reading → telemetry → OTA check & download |
| **V2.0** (separate `main.rs`) | New WiFi (proof of update), adds `sensor_timestamp`, SNTP sync |

---

## **OTA Update Workflow**

1. **V1.0 running** → polls `shared attributes` on ThingsBoard
2. New firmware detected → status: `DOWNLOADING`
3. Downloads chunks via MQTT (`v2/fw/response/...`)
4. Validates **SHA256 checksum**
5. Switches boot partition → `esp_restart()`
6. **V2.0 boots** → status: `UPDATED`

> **Automatic rollback** on checksum failure or boot error.

---

## **ThingsBoard Dashboards**

### 1. **Data Dashboard**
- Real-time charts: Temp, Humidity, Pressure, CO₂
- Map widget (latitude/longitude)
- Latest values table

### 2. **OTA Dashboard**
- Status: `IDLE`, `DOWNLOADING`, `VERIFYING`, `UPDATED`, `FAILED`
- Download progress (%)
- Current firmware version

---

## **Key Files**

| File | Purpose |
|------|--------|
| `main.rs` | V1.0 firmware (with OTA client) |
| `v2_main.rs` | V2.0 firmware (timestamp + new WiFi) |
| `Cargo.toml` | Rust dependencies |
| `partitions.csv` | Flash memory layout |
| `flash.sh` | Build & flash initial firmware |
| `build-ota.sh` | Generate `.bin` for OTA upload |

---

## **How to Run**

### 1. **Flash Initial Firmware (V1.0)**

```bash
chmod +x flash.sh
./flash.sh
```

> Update `PORT` in script (`/dev/ttyACM0` or `/dev/ttyUSB0`)

---

### 2. **Build & Upload OTA Firmware (V2.0)**

```bash
chmod +x build-ota.sh
./build-ota.sh
```

> Output: `firmware/week-1-YYYYMMDD-HHMMSS.bin`  
> Upload `.bin` to **ThingsBoard → Device Profiles → Weather Station → Firmware**

---

### 3. **ThingsBoard Setup**

1. Create **Device**: `Weather Station ESP32`
2. Use **Access Token**: `eprtrartn5tpdw7oq38f`
3. Enable **OTA** in Device Profile
4. Upload `.bin` to **OTA Packages**
5. Create **2 Dashboards**:
   - One for sensor data
   - One for OTA status (`fw_state`, `progress`)

---

## **Latency Measurement (V2.0)**

```json
"sensor_timestamp": "2025-10-23 14:30:45"
```

> Compare with dashboard display time → calculate **network + processing latency**

---

## **System Advantages**

| Aspect | Advantage |
|------|----------|
| **Efficiency** | Rust `no_std` → minimal binary size |
| **Security** | Memory safety + SHA256 verification |
| **Reliability** | Dual partitions + rollback |
| **Flexibility** | Cable-free updates |
| **Observability** | Real-time dashboards + logging |

---

## **Dependencies (`Cargo.toml`)**

```toml
[dependencies]
esp-idf-sys = "0.36"
esp-idf-hal = "0.45"
esp-idf-svc = "0.51"
bme280 = { version = "0.5", features = ["sync"] }
sha2 = "0.10"
serde_json = { version = "1.0", features = ["alloc"] }
heapless = "0.8"
anyhow = "1.0"
log = "0.4"
```

---

## **Development Notes**

- Requires **Rust 1.77+**
- Target: `xtensa-esp32s3-espidf`
- Install `espflash` and `cargo-esp`
- WiFi credentials are hardcoded (update as needed)

---

## **License**

```
MIT License © 2025 Andik Putra Nazwana & Rany Surya Oktavia
```

---

## **Contact**

- **Email**: andiknazwana04@gmail.com  
- **GitHub**: [github.com/andiknazwana](https://github.com/andiknazwana)  
- **ThingsBoard Demo**: [demo.thingsboard.io](https://demo.thingsboard.io) *(use your token)*

---

> **"Embedded Rust + OTA = The Future of Secure, Efficient IoT"**  
> — *Andik Putra Nazwana*

---

**© 2025 Department of Instrumentation Engineering – ITS**  
*IoT Course Project – Odd Semester 2025/2026*
