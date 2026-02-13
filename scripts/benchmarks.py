import time
import asyncio
import subprocess
from typing import Any

from pybox.box import PyBox
from mcp_run_python import code_sandbox
from pydantic_monty import Monty




def pybox_startup():    
    start = time.perf_counter()
    box = PyBox()
    box.init_local("1")
    diff = time.perf_counter() - start
    print(f'PyBox cold start time: {(diff * 1000):.3f} millisecond')


def pybox_context():    
    box = PyBox()
    start = time.perf_counter()
    box.init_local("1")
    diff = time.perf_counter() - start
    print(f'PyBox context time: {(diff * 1000):.3f} millisecond')



def pybox_code():
    box = PyBox()
    box.init_local("1")
    start = time.perf_counter()
    box.exec("1+1","1")
    diff = time.perf_counter() - start
    print(f'PyBox exec time: {(diff * 1000):.3f} millisecond')



def pyodide_startup():
    async def run() -> Any:
        async with code_sandbox() as sandbox:
            return
    
    start = time.perf_counter()
    asyncio.run(run())
    diff = time.perf_counter() - start
    print(f'Pyodide cold start time: {(diff * 1000):.3f} millisecond')



def pyodide_code():

    start = None
    diff = None
    async def run() -> Any:
        async with code_sandbox() as sandbox:
            nonlocal start,diff
            start = time.perf_counter()
            await sandbox.eval("1+1")
            diff = time.perf_counter() - start
            return
    
    
    result = asyncio.run(run())
    
    print(f'Pyodide exec time: {(diff * 1000):.3f} millisecond')



def monty_startup():
    start = time.perf_counter()
    Monty('')
    diff = time.perf_counter() - start
    print(f'Monty cold start time: {(diff * 1000):.3f} milliseconds')

def monty_code():
    code = Monty('1+1')
    start = time.perf_counter()
    code.run()
    diff = time.perf_counter() - start
    print(f'Monty exec time: {(diff * 1000):.3f} milliseconds')


def run_docker():
    start = time.perf_counter()
    result = subprocess.run(
        ['docker', 'run', '--rm', 'python:3.14-alpine', 'python', '-c', 'print(1+1)'],
        capture_output=True,
        text=True,
        encoding='utf-8',
    )
    diff = time.perf_counter() - start
    output = result.stdout.strip()
    assert output == '2', f'Unexpected result: {output!r}'
    print(f'Docker cold start time: {(diff * 1000):.3f} milliseconds')


def run_wasmer():
    # requires wasmer to be installed, see https://docs.wasmer.io/install
    start = time.perf_counter()
    result = subprocess.run(
        ['wasmer', 'run', 'python/python', '--', '-c', 'print(1+1)'],
        capture_output=True,
        text=True,
        encoding='utf-8',
    )
    diff = time.perf_counter() - start
    output = result.stdout.strip()
    assert output == '2', f'Unexpected result: {output!r}'
    print(f'Wasmer cold start time: {(diff * 1000):.3f} milliseconds')


def run_subprocess_python():
    start = time.perf_counter()
    result = subprocess.run(
        ['python', '-c', 'print(1+1)'],
        capture_output=True,
        text=True,
        encoding='utf-8',
    )
    diff = time.perf_counter() - start
    output = result.stdout.strip()
    assert output == '2', f'Unexpected result: {output!r}'
    print(f'Subprocess Python cold start time: {(diff * 1000):.3f} milliseconds')


if __name__ == '__main__':
    pybox_startup()
    pybox_startup()
    pybox_code()
    pybox_context()
    monty_startup()
    monty_code()
    pyodide_startup()
    pyodide_code()
    run_docker()
    # run_wasmer()
    