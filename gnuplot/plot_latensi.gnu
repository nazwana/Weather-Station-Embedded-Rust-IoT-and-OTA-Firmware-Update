reset
set terminal pngcairo size 800,600
set output 'grafik_latensi_3okt.png'
set title 'Analisis Latensi ESP32 ke Thingsboard (3 Oktober 2025)'
set xlabel 'Waktu (Jam:Menit)'
set ylabel 'Latensi (detik)'
set grid
set format y "%.2f"
set xdata time
set timefmt "%H:%M:%S"
set format x "%H:%M"  # Hanya tampilkan jam dan menit
set xtics rotate by -45
plot 'latensi_data_3okt.dat' using 1:2 with lines linewidth 2 linecolor rgb "blue" title 'Thingsboard', \
     'latensi_data_3okt.dat' using 1:3 with lines linewidth 2 linecolor rgb "red" title 'ESP32'
set output