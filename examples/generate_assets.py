from PIL import Image, ImageDraw
import os

# Ensure directories exist
os.makedirs("examples/oggy", exist_ok=True)
os.makedirs("examples/jerry", exist_ok=True)
os.makedirs("examples/tom", exist_ok=True)

def create_img(path, w, h, color):
    img = Image.new("RGBA", (w, h), color)
    img.save(path)
    print(f"Created {path}")

# Larger background for camera pan (Video is 200x200, so 400x400 gives room to pan)
create_img("examples/bg.png", 400, 400, (0, 0, 255, 255)) # Blue

# ==========================
# 1. OGGY (Red/Yellow Theme)
# ==========================
create_img("examples/oggy/body.png", 40, 60, (255, 0, 0, 255))
create_img("examples/oggy/part_9.png", 30, 30, (255, 255, 0, 255))
create_img("examples/oggy/part_10.png", 15, 15, (0, 255, 0, 255))
create_img("examples/oggy/part_6.png", 10, 30, (0, 128, 0, 255))
create_img("examples/oggy/part_8.png", 15, 15, (255, 0, 255, 255))
create_img("examples/oggy/part_5.png", 10, 30, (128, 0, 128, 255))
create_img("examples/oggy/part_3.png", 12, 40, (165, 42, 42, 255))
create_img("examples/oggy/part_4.png", 12, 40, (0, 0, 0, 255))

# Faces
create_img("examples/oggy/face_eyes_happy.png", 10, 5, (0, 0, 0, 255))
create_img("examples/oggy/face_eyes_sad.png", 10, 5, (0, 0, 255, 255))
create_img("examples/oggy/face_mouth_open.png", 8, 8, (255, 100, 100, 255))
create_img("examples/oggy/face_mouth_closed.png", 8, 4, (100, 0, 0, 255))

# ==========================
# 2. JERRY (Green/Cyan Theme)
# ==========================
create_img("examples/jerry/body.png", 30, 40, (0, 255, 0, 255))
create_img("examples/jerry/part_9.png", 20, 20, (0, 255, 255, 255))
create_img("examples/jerry/part_10.png", 10, 10, (0, 128, 0, 255))
create_img("examples/jerry/part_6.png", 8, 20, (0, 64, 0, 255))
create_img("examples/jerry/part_8.png", 10, 10, (0, 128, 0, 255))
create_img("examples/jerry/part_5.png", 8, 20, (0, 64, 0, 255))
create_img("examples/jerry/part_3.png", 10, 20, (0, 0, 0, 255))
create_img("examples/jerry/part_4.png", 10, 20, (0, 0, 0, 255))

# Faces
create_img("examples/jerry/face_eyes_happy.png", 8, 4, (0, 0, 0, 255))
create_img("examples/jerry/face_mouth_open.png", 6, 6, (255, 100, 100, 255))

# ==========================
# 3. TOM (Gray/White Theme)
# ==========================
create_img("examples/tom/body.png", 35, 50, (128, 128, 128, 255))
create_img("examples/tom/part_9.png", 25, 25, (200, 200, 200, 255))
create_img("examples/tom/part_10.png", 12, 12, (50, 50, 50, 255))
create_img("examples/tom/part_6.png", 9, 25, (30, 30, 30, 255))
create_img("examples/tom/part_8.png", 12, 12, (50, 50, 50, 255))
create_img("examples/tom/part_5.png", 9, 25, (30, 30, 30, 255))
create_img("examples/tom/part_3.png", 11, 30, (20, 20, 20, 255))
create_img("examples/tom/part_4.png", 11, 30, (20, 20, 20, 255))

# Faces
create_img("examples/tom/face_eyes_happy.png", 9, 5, (0, 0, 0, 255))
create_img("examples/tom/face_mouth_open.png", 7, 7, (255, 100, 100, 255))

print("All assets generated successfully!")
