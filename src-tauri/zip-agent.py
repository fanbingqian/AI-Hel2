"""Create hermes-agent.zip using Python's zipfile (handles locked files)."""
import zipfile, os, sys
src = os.path.join(os.path.dirname(os.path.abspath(__file__)), "hermes-agent")
dst = os.path.join(os.path.dirname(os.path.abspath(__file__)), "hermes-agent.zip")
zf = zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED)
count = 0
for root, dirs, files in os.walk(src):
    for f in files:
        fp = os.path.join(root, f)
        try:
            zf.write(fp, fp)
            count += 1
        except Exception as e:
            print(f"SKIP: {fp} - {e}", file=sys.stderr)
zf.close()
print(f"Created {dst} with {count} files")
