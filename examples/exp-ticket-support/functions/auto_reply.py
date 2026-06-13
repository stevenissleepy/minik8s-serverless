import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        category = event.get("category", "general")

        replies = {
            "refund": "Your refund request has been received.",
            "technical": "We sent password and troubleshooting steps.",
            "complaint": "We recorded your complaint and will follow up.",
            "general": "We received your ticket.",
        }
        result = {
            **event,
            "action": "auto-reply",
            "reply": replies.get(category, replies["general"]),
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
