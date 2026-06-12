import json
import time


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        seconds = float((event or {}).get("sleep", 2))
        time.sleep(seconds)
        payload = json.dumps({
            "function": scope["minik8s"]["name"],
            "slept": seconds,
        }).encode("utf-8")
        await send({
            "type": "http.response.start",
            "status": 200,
            "headers": [[b"content-type", b"application/json"]],
        })
        await send({"type": "http.response.body", "body": payload})
