import time
import sys

def main():
    print("Starting RAM leaker...")
    leaked = []
    chunk_size = 100 * 1024 * 1024  # 100 MB chunks
    
    while True:
        try:
            leaked.append(bytearray(chunk_size))
            total_mb = len(leaked) * 100
            print(f"Allocated {total_mb} MB")
            time.sleep(0.5)
        except MemoryError:
            print("Out of memory in python!")
            break

if __name__ == "__main__":
    main()
