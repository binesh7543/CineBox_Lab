import zlib, struct, os

def create_png(filename, width, height, rgba):
    os.makedirs(os.path.dirname(filename), exist_ok=True)
    def chunk(type, data):
        return struct.pack('>I', len(data)) + type + data + struct.pack('>I', zlib.crc32(type + data))
    
    header = b'\x89PNG\r\n\x1a\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
    raw_data = b''
    for _ in range(height):
        raw_data += b'\x00' + bytes(rgba) * width
    idat = chunk(b'IDAT', zlib.compress(raw_data))
    iend = chunk(b'IEND', b'')
    
    with open(filename, 'wb') as f:
        f.write(header + ihdr + idat + iend)

# Generate solid color PNGs
create_png("examples/bg.png", 200, 200, [0, 0, 255, 255])          # Blue BG
create_png("examples/oggy/body.png", 40, 40, [255, 0, 0, 255])     # Red Body
create_png("examples/oggy/part_10.png", 10, 10, [0, 255, 0, 255]) # Green Part
print("Example assets generated successfully.")
