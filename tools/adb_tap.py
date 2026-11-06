#!/usr/bin/env python3
"""Tap a button on an Android device by its text or content description (uiautomator dump).

    tools/adb_tap.py <adb-serial> <text>     e.g. tools/adb_tap.py QV781CEX8K record

Screen coordinates change with the status line height, so tapping by text is the reliable way.
"""
import subprocess, sys, re, xml.etree.ElementTree as ET
dev, text = sys.argv[1], sys.argv[2]
subprocess.run(["adb","-s",dev,"shell","uiautomator","dump","/sdcard/ui.xml"],capture_output=True)
xml = subprocess.run(["adb","-s",dev,"shell","cat","/sdcard/ui.xml"],capture_output=True,text=True).stdout
root = ET.fromstring(xml[xml.index("<"):])
for n in root.iter("node"):
    if (n.get("text") or "").strip().lower() == text.lower() or (n.get("content-desc") or "").strip().lower() == text.lower():
        x1,y1,x2,y2 = map(int, re.findall(r"\d+", n.get("bounds")))
        cx, cy = (x1+x2)//2, (y1+y2)//2
        subprocess.run(["adb","-s",dev,"shell","input","tap",str(cx),str(cy)])
        print("tapped", text, cx, cy); sys.exit(0)
print("not found:", text); sys.exit(1)
