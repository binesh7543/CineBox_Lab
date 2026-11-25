from PIL import Image, ImageDraw
import os

# Ensure directory exists
os.makedirs("examples/oggy", exist_ok=True)

# Helper to create solid color image
def create_img(path, w, h, color):
    img = Image.new("RGBA", (w, h), color)
    img.save(path)
    print(f"Created {path}")

# Background
create_img("examples/bg.png", 200, 200, (0, 0, 255, 255)) # Blue

# Body (Red)
create_img("examples/oggy/body.png", 40, 60, (255, 0, 0, 255))

# Head (Yellow)
create_img("examples/oggy/part_9.png", 30, 30, (255, 255, 0, 255))

# Right Arm parts (Green shoulder, Dark Green arm)
create_img("examples/oggy/part_10.png", 15, 15, (0, 255, 0, 255))
create_img("examples/oggy/part_6.png", 10, 30, (0, 128, 0, 255))

# Left Arm parts (Magenta shoulder, Purple arm)
create_img("examples/oggy/part_8.png", 15, 15, (255, 0, 255, 255))
create_img("examples/oggy/part_5.png", 10, 30, (128, 0, 128, 255))

# Legs (Brown, Black)
create_img("examples/oggy/part_3.png", 12, 40, (165, 42, 42, 255))
create_img("examples/oggy/part_4.png", 12, 40, (0, 0, 0, 255))

print("All assets generated successfully!")
