from PIL import Image
from collections import deque

ico_path = r'C:\Users\hrist\lol-account-manager-tauri\src-tauri\icons\icon.ico'

img = Image.open(ico_path).convert('RGBA')
pixels = img.load()
w, h = img.size

def color_dist(a, b):
    return max(abs(a[i] - b[i]) for i in range(3))

def flood_fill_transparent(img, pixels, start_corners, tolerance=30):
    bg_color = pixels[start_corners[0]]
    visited = set()
    queue = deque(start_corners)
    for c in start_corners:
        visited.add(c)

    while queue:
        x, y = queue.popleft()
        if color_dist(pixels[x, y], bg_color) <= tolerance:
            pixels[x, y] = (0, 0, 0, 0)
            for dx, dy in [(-1,0),(1,0),(0,-1),(0,1)]:
                nx, ny = x+dx, y+dy
                if 0 <= nx < w and 0 <= ny < h and (nx, ny) not in visited:
                    visited.add((nx, ny))
                    queue.append((nx, ny))

corners = [(0,0), (w-1,0), (0,h-1), (w-1,h-1)]
flood_fill_transparent(img, pixels, corners, tolerance=20)

img.save(ico_path, format='ICO', sizes=[(256, 256)])
print("Icon saved with transparent background.")
