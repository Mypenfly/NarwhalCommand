//!@Open: ./tests/data/python_app.py
//!@Location:Block
def handle_get(task_id: str) -> dict:
//!@New:
def handle_delete(task_id: str) -> dict:
    """Handle DELETE /tasks/{id} — remove a task."""
    success = _manager.delete_task(task_id)
    if not success:
        return {"error": "Task not found"}, 404
    return {"status": "deleted"}, 200

...
//!@Off:Open
