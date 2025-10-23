import pandas as pd

try:

    file_path = '/home/nazwana/Documents/IoT/week-1/gnuplot/timestamps (3 okt).csv'
    df = pd.read_csv(file_path, sep=',')

    if 'Timestamp' not in df.columns or 'Timestamp (ESP32)' not in df.columns:
        raise KeyError("Kolom 'Timestamp' atau 'Timestamp (ESP32)' tidak ditemukan di file CSV.")

    df['Timestamp'] = pd.to_datetime(df['Timestamp'].str.strip('"'))
    df['Timestamp (ESP32)'] = pd.to_datetime(df['Timestamp (ESP32)'].str.strip('"'))
    df['menit'] = df['Timestamp'].dt.strftime('%H:%M')

    df['latensi'] = (df['Timestamp'] - df['Timestamp (ESP32)']).dt.total_seconds()

    df_avg = df.groupby('menit').agg({
        'latensi': 'mean',
        'Timestamp': 'first'  
    }).reset_index()

    df_avg['waktu'] = df_avg['Timestamp'].dt.strftime('%H:%M:%S')

    df_avg['baseline'] = 0.0

    output_path = '/home/nazwana/Documents/IoT/week-1/gnuplot/latensi_data_3okt.dat'
    df_avg[['waktu', 'latensi', 'baseline']].to_csv(output_path, sep='\t', index=False)
    print(f"File 'latensi_data.dat' berhasil dibuat di {output_path}")

except FileNotFoundError:
    print("File timestamps.csv tidak ditemukan di direktori yang ditentukan.")
except KeyError as e:
    print(f"Error: {e}")
except Exception as e:
    print(f"Terjadi error: {e}")