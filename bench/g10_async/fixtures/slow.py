#!/Users/spica4345/Projects/pruv/.venv/bin/python
import os, time
os.write(1,b"on")
time.sleep(0.02)
os.write(1,b"e\ntwo\n")
os.write(2,b"e"*1200000)
time.sleep(10)
