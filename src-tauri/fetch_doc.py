import urllib.request

url = "https://docs.langchain.com/oss/python/langchain/event-streaming.md"
data = urllib.request.urlopen(url).read().decode("utf-8")
with open("event-streaming-doc.md", "w", encoding="utf-8") as f:
    f.write(data)
print("len:", len(data))
