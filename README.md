# **Weather Station with ESP32‑S3 & OTA Firmware Update (ThingsBoard)**  
### *Internet of Things (IoT) Course Project*  
**Department of Instrumentation Engineering, Vocational Faculty – Institut Teknologi Sepuluh Nopember (ITS)**  

---

## **Developers**
| **Supervisor** | **Students** |
|----------------|--------------|
| **Ahmad Radhy, S.Si., M.Si.** | **Andik Putra Nazwana**<br>**ardo Surya Oktavia** |

---

## **Abstract**
This **Weather Station** enables **real‑time environmental monitoring** using the **ESP32‑S3** programmed in **Rust `no_std`**.  
Data from **BME280** (temperature, humidity, pressure) and **MQ‑135** (CO₂) is sent to **ThingsBoard Cloud** via **MQTT**.

**Highlight:** **Over‑the‑Air (OTA) Firmware Updates** with dual‑partition safety, SHA‑256 checksum verification, and automatic rollback – no physical access needed.

---

## **System Architecture**
![System Architecture](https://raw.githubusercontent.com/nazwana/Weather-Station-Embedded-Rust-IoT-and-OTA-Firmware-Update/main/documentation/21.%20Arsitektur%20Sistem.jpeg)

---

## **Key Features**

| Feature | Description |
|---------|-------------|
| **Real‑time Telemetry** | Temperature, humidity, pressure, CO₂, static GPS |
| **OTA Firmware Update** | Remote deployment via ThingsBoard |
| **Dual OTA Partitions** | `ota_0` & `ota_1` – safe updates + rollback |
| **Checksum Verification** | SHA‑256 before boot |
| **Dual Dashboards** | 1. Sensor data 2. OTA status |
| **Rust `no_std`** | Minimal footprint & memory safety |
| **RTC Timestamp (V2.0)** | End‑to‑end‑latency measurement |
| **SNTP Sync** | Accurate internet time |

---

## **Hardware Components**

| Component | Function |
|-----------|----------|
| **ESP32‑S3** | MCU with Wi‑Fi, dual‑core |
| **BME280** | I²C sensor – temp, humidity, pressure |
| **MQ‑135** | ADC sensor – CO₂ / air quality |
| **Wi‑Fi** | Internet connectivity |

---

## **Software Stack**

- **Language** – Rust `no_std`  
- **Framework** – `esp-idf-hal`, `esp-idf-svc`  
- **IoT Platform** – [ThingsBoard Cloud](https://thingsboard.cloud)  
- **Protocol** – MQTT  
- **OTA Service** – ThingsBoard Firmware OTA  
- **Build Tools** – `cargo`, `espflash`, `embuild`

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

> **Dual OTA Slots** – safe updates with rollback.

---

## **Firmware Versions**

| Version | Key Features |
|---------|--------------|
| **V1.0** (`main.rs`) | Sensor read → telemetry → OTA polling & download |
| **V2.0** (separate `main.rs`) | New Wi‑Fi (proof), `sensor_timestamp`, SNTP sync |

---

## **OTA Update Workflow**

1. **V1.0** polls `shared attributes` on ThingsBoard  
2. New firmware → status **`DOWNLOADING`**  
3. Chunk download via MQTT (`v2/fw/response/...`)  
4. **SHA‑256** checksum validation  
5. Switch partition → `esp_restart()`  
6. **V2.0** boots → status **`UPDATED`**

> *Automatic rollback* on checksum or boot failure.

---

## **Demo Dashboards (ThingsBoard)**

### 1. **Weather Data Dashboard**
![Weather Data Dashboard](https://raw.githubusercontent.com/nazwana/Weather-Station-Embedded-Rust-IoT-and-OTA-Firmware-Update/main/documentation/11.%20Dashboard%20Data%20(Thingsboard).png)

*Cards, real‑time line charts, latest‑value table, and a map widget showing device location.*

### 2. **OTA Firmware Status Dashboard**
![OTA Updated Dashboard](https://raw.githubusercontent.com/nazwana/Weather-Station-Embedded-Rust-IoT-and-OTA-Firmware-Update/main/documentation/20.%20Dashboard%20OTA%20Updated%20(Thingsboard).png)

*Shows current firmware version, OTA state (`UPDATED`, `DOWNLOADING`, …) and download progress.*

---

## **Key Files**

| File | Purpose |
|------|---------|
| `main.rs` | V1.0 firmware (OTA client) |
| `v2_main.rs` | V2.0 firmware (timestamp + new Wi‑Fi) |
| `Cargo.toml` | Rust dependencies |
| `partitions.csv` | Flash layout |
| `flash.sh` | Build & flash initial firmware |
| `build-ota.sh` | Generate `.bin` for OTA upload |

---

## **How to Run**

### 1. Flash Initial Firmware (V1.0)

```bash
chmod +x flash.sh
./flash.sh
```

> Edit `PORT` (`/dev/ttyACM0` or `/dev/ttyUSB0`) as needed.

### 2. Build & Upload OTA Firmware (V2.0)

```bash
chmod +x build-ota.sh
./build-ota.sh
```

> Output: `firmware/week-1-YYYYMMDD-HHMMSS.bin`  
> Upload to **ThingsBoard → Device Profiles → Weather Station → Firmware**

### 3. ThingsBoard Setup

1. **Device** – `Weather Station ESP32`  
2. **Access Token** – `eprtrartn5tpdw7oq38f`  
3. Enable **OTA** in Device Profile  
4. Upload `.bin` to **OTA Packages**  
5. Create **2 Dashboards** (see screenshots above)

---

## **Latency Measurement (V2.0)**

```json
"sensor_timestamp": "2025-10-23 14:30:45"
```

Compare with dashboard time → **network + processing latency**.

---

## **System Advantages**

| Aspect | Advantage |
|--------|-----------|
| **Efficiency** | Rust `no_std` → tiny binary |
| **Security** | Memory safety + SHA‑256 |
| **Reliability** | Dual partitions + rollback |
| **Flexibility** | Cable‑free updates |
| **Observability** | Real‑time dashboards + logs |

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

- **Rust ≥ 1.77**  
- Target: `xtensa-esp32s3-espidf`  
- Install `espflash` & `cargo-esp`  
- Wi‑Fi credentials are hard‑coded – change as required

---

## **License**

```
MIT License © 2025 Andik Putra Nazwana & Rany Surya Oktavia
```

---

## **Contact**

- **Email**: andiknazwana04@gmail.com  
- **GitHub**: [github.com/nazwana](https://github.com/nazwana)  
- **ThingsBoard Demo**: [demo.thingsboard.io](https://demo.thingsboard.io) *(use your token)*

---

> **"Embedded Rust + OTA = The Future of Secure, Efficient IoT"**  
> — *Andik Putra Nazwana*

---

**© 2025 Department of Instrumentation Engineering – ITS**  
*IoT Course Project – Odd Semester 2025/2026*
```
