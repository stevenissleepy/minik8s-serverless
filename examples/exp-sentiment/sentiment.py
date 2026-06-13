import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        text = (event or {}).get("text", "")
        score = 1 if "good" in text.lower() else -1
        result = {
            "text": text,
            "label": "positive" if score > 0 else "negative",
            "score": score,
            "function": scope["minik8s"]["name"],
        }
        payload = json.dumps(result).encode("utf-8")
        await send({
            "type": "http.response.start",
            "status": 200,
            "headers": [[b"content-type", b"application/json"]],
        })
        await send({"type": "http.response.body", "body": payload})
