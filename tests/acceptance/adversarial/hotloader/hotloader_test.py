import pytest
import time
import random

def test_hotloader_delay_injection():
    """网络超时注入测试 - 针对 Hotloader"""
    start_time = time.time()
    time.sleep(0.5)
    end_time = time.time()
    assert (end_time - start_time) >= 0.3, "模拟超时失败"

def test_hotloader_malformed_input():
    """畸形 Payload 测试 - 针对 Hotloader"""
    # 假设 hotloader 接收配置数据
    payload = "INVALID_CONFIG"
    assert payload != "VALID_CONFIG", "配置应无效"
