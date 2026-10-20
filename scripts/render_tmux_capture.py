#!/usr/bin/env python3
import sys
from PIL import Image, ImageDraw, ImageFont

def render_tmux_capture_to_png(txt_path, png_path):
    with open(txt_path, 'r', encoding='utf-8') as f:
        lines = [line.rstrip('\r\n') for line in f.readlines()]

    font_path = "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"
    font_size = 15
    try:
        font = ImageFont.truetype(font_path, font_size)
    except Exception:
        font = ImageFont.load_default()

    line_h = 24
    max_w = 800
    for l in lines:
        try:
            bbox = font.getbbox(l)
            w = bbox[2] - bbox[0]
            if w > max_w:
                max_w = w
        except Exception:
            pass

    img_w = max_w + 60
    img_h = len(lines) * line_h + 40

    img = Image.new('RGB', (img_w, img_h), color=(18, 16, 14))
    draw = ImageDraw.Draw(img)

    y = 20
    for line in lines:
        x = 30
        text_color = (203, 200, 195)
        if "▌" in line:
            draw.rectangle([(25, y - 2), (img_w - 25, y + line_h - 4)], fill=(40, 37, 32))
            text_color = (232, 229, 224)
        elif "提问" in line:
            # 明亮的陶土色 Terracotta Bright
            text_color = (242, 142, 102)
        elif "·" in line and ("移动" in line or "确认" in line or "提交" in line):
            text_color = (124, 118, 110)
        elif "等待你的选择" in line:
            text_color = (162, 155, 146)

        draw.text((x, y), line, font=font, fill=text_color)
        y += line_h

    img.save(png_path)
    print(f"Saved screenshot with proper CJK font to {png_path}")

if __name__ == '__main__':
    render_tmux_capture_to_png(sys.argv[1], sys.argv[2])
