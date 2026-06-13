import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        risk = int(event.get("risk", 0))

        result = {
            **event,
            "action": "human-escalate",
            "priority": "p1" if risk >= 90 else "p2",
            "queue": "vip-support" if event.get("user_level") == "vip" else "support",
            "handled_by": scope["minik8s"]["name"],
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
