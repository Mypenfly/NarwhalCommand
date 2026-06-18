"""
A simple Flask-like web application for managing a task list.

This module provides a TaskManager class with CRUD operations
and a REST API layer built on top of it.
"""

import json
import uuid
from datetime import datetime, timezone
from dataclasses import dataclass, field
from typing import Optional


# ---------------------------------------------------------------------------
# Data Models
# ---------------------------------------------------------------------------

@dataclass
class Task:
    """A single task in the system."""

    title: str
    description: str = ""
    completed: bool = False
    task_id: str = field(default_factory=lambda: str(uuid.uuid4()))
    created_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())

    def mark_completed(self) -> None:
        """Mark this task as completed."""
        self.completed = True

    def to_dict(self) -> dict:
        """Convert the task to a JSON-serializable dictionary."""
        return {
            "task_id": self.task_id,
            "title": self.title,
            "description": self.description,
            "completed": self.completed,
            "created_at": self.created_at,
        }


# ---------------------------------------------------------------------------
# Core Service
# ---------------------------------------------------------------------------

class TaskManager:
    """Manages a collection of tasks with in-memory storage."""

    def __init__(self) -> None:
        """Initialize an empty task manager."""
        self._tasks: dict[str, Task] = {}
        self._deleted_count: int = 0

    def create_task(self, title: str, description: str = "") -> Task:
        """Create a new task and add it to the collection."""
        task = Task(title=title, description=description)
        self._tasks[task.task_id] = task
        return task

    def get_task(self, task_id: str) -> Optional[Task]:
        """Retrieve a task by its unique identifier."""
        return self._tasks.get(task_id)

    def list_tasks(self, include_completed: bool = True) -> list[Task]:
        """Return all tasks, optionally filtering out completed ones."""
        tasks = list(self._tasks.values())
        if not include_completed:
            tasks = [t for t in tasks if not t.completed]
        return sorted(tasks, key=lambda t: t.created_at, reverse=True)

    def update_task(
        self, task_id: str, title: Optional[str] = None,
        description: Optional[str] = None,
    ) -> Optional[Task]:
        """Update task fields. Returns None if task not found."""
        task = self._tasks.get(task_id)
        if task is None:
            return None
        if title is not None:
            task.title = title
        if description is not None:
            task.description = description
        return task

    def complete_task(self, task_id: str) -> bool:
        """Mark a task as completed. Returns False if not found."""
        task = self._tasks.get(task_id)
        if task is None:
            return False
        task.mark_completed()
        return True

    def delete_task(self, task_id: str) -> bool:
        """Remove a task from the collection."""
        if task_id in self._tasks:
            del self._tasks[task_id]
            self._deleted_count += 1
            return True
        return False

    def count_by_status(self) -> dict:
        """Count tasks grouped by completion status."""
        result = {"total": 0, "completed": 0, "pending": 0}
        for task in self._tasks.values():
            result["total"] += 1
            if task.completed:
                result["completed"] += 1
            else:
                result["pending"] += 1
        return result


# ---------------------------------------------------------------------------
# API Handlers (simulated)
# ---------------------------------------------------------------------------

_manager = TaskManager()


def handle_create(request_body: dict) -> dict:
    """Handle POST /tasks — create a new task."""
    title = request_body.get("title", "").strip()
    if not title:
        return {"error": "Title is required"}, 400
    description = request_body.get("description", "")
    task = _manager.create_task(title, description)
    return task.to_dict(), 201


def handle_list(query_params: dict) -> dict:
    """Handle GET /tasks — list all tasks."""
    include_completed = query_params.get("include_completed", "true").lower() == "true"
    tasks = _manager.list_tasks(include_completed)
    return {"tasks": [t.to_dict() for t in tasks], "count": len(tasks)}, 200


def handle_get(task_id: str) -> dict:
    """Handle GET /tasks/{id} — get a single task."""
    task = _manager.get_task(task_id)
    if task is None:
        return {"error": "Task not found"}, 404
    return task.to_dict(), 200


def handle_complete(task_id: str) -> dict:
    """Handle POST /tasks/{id}/complete — mark task as done."""
    success = _manager.complete_task(task_id)
    if not success:
        return {"error": "Task not found"}, 404
    return {"status": "completed"}, 200


def router(path: str, method: str, body: Optional[dict] = None) -> tuple[dict, int]:
    """Simple URL router dispatching to the correct handler."""
    parts = path.strip("/").split("/")

    if path == "/tasks" and method == "POST":
        return handle_create(body or {})
    if path == "/tasks" and method == "GET":
        return handle_list({})
    if len(parts) == 2 and parts[0] == "tasks" and method == "GET":
        return handle_get(parts[1])
    if len(parts) == 3 and parts[0] == "tasks" and parts[2] == "complete" and method == "POST":
        return handle_complete(parts[1])

    return {"error": "Not found"}, 404
