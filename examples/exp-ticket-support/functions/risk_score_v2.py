import json
import socket
import time


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        sleep_ms = int(event.get("sleep_ms", 0) or 0)
        if sleep_ms > 0:
            time.sleep(sleep_ms / 1000)

        result = {
            **event,
            "risk": 100,
            "risk_level": "high",
            "decision": "human",
            "model_version": "risk-v2",
            "instance": socket.gethostname(),
            "scored_by": scope["minik8s"]["name"],
        }
        await respond(send, result)


async def respond(send, result):
    payload = json.dumps(result).encode("utf-8")
    await send({
        "type": "http.response.start",
        "status": 200,
        "headers": [[b"content-type", b"application/json"]],
    })
    await send({"type": "http.response.body", "body": payload})
