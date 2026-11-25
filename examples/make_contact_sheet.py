import os
from PIL import Image

# 15 frames ko ek 5x3 grid (contact sheet) mein jodne ke liye
frames_dir = "examples/frames"
output_path = "examples/contact_sheet.png"

frames = []
for i in range(15):
    path = os.path.join(frames_dir, f"frame_{i:03d}.png")
    if os.path.exists(path):
        frames.append(Image.open(path))
    else:
        print(f"Warning: {path} not found")

if not frames:
    print("No frames found to make contact sheet.")
    exit(1)

# Maan lo frames 200x200 ke hain
w, h = frames[0].size
cols = 5
rows = 3

# Naya blank image banayo
sheet = Image.new("RGBA", (w * cols, h * rows))

for i, frame in enumerate(frames):
    x = (i % cols) * w
    y = (i // cols) * h
    sheet.paste(frame, (x, y))

sheet.save(output_path)
print(f"Contact sheet saved to {output_path}")
