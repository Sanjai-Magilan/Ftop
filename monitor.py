import psutil
import time
import os

def get_stats():
    cpu = psutil.cpu_percent(interval=1)
    mem = psutil.virtual_memory()
    disk = psutil.disk_usage('/')
    
    return cpu, mem.percent, disk.percent

while True:
    os.system("clear")
    
    cpu, mem, disk = get_stats()
    
    print("=== SYSTEM MONITOR ===")
    print(f"CPU Usage: {cpu}%")
    print(f"Memory Usage: {mem}%")
    print(f"Disk Usage: {disk}%")
    
    time.sleep(1)
