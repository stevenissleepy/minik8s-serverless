import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        text = str(event.get("text", ""))
        lower = text.lower()

        if any(word in lower for word in ["refund", "money", "charge"]):
            category = "refund"
        elif any(word in lower for word in ["password", "login", "error", "bug"]):
            category = "technical"
        elif any(word in lower for word in ["angry", "complaint", "terrible"]):
            category = "complaint"
        else:
            category = "general"

        result = {
            **event,
            "category": category,
            "classified_by": scope["minik8s"]["name"],
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
